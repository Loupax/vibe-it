//! web: the browser's shell (web/TODO.txt C2, Architecture C in web/DECISIONS.md).
//!
//! A small Wayland compositor (smithay) shown as one window on the user's desktop (winit:
//! a Wayland or an X11 window). It starts the engine host (web-host-cef) with
//! WAYLAND_DISPLAY set to its own socket and drives it over a control socket it listens
//! on (web/PROTOCOL.md). The page's window fills the shell window minus the status bar.

mod args;
mod bookmarks;
mod buffers;
mod clipboard;
mod completion;
mod config;
mod control;
mod dmabuf;
mod external;
mod forget;
mod history;
mod host;
mod ime;
mod input;
mod keymap;
mod keys;
mod layout;
mod lock;
mod pacing;
mod pairing;
mod picker;
mod popups;
mod probe;
mod protocol;
mod registers;
mod render;
mod selection;
mod state;
mod statusbar;
mod sync;
mod syncobj;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::winit::{self, WinitEvent};
use smithay::desktop::{PopupManager, Space};
use smithay::input::pointer::CursorImageStatus;
use smithay::input::SeatState;
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{EventLoop, Interest, Mode as CalloopMode, PostAction};
use smithay::reexports::wayland_server::Display;
use smithay::reexports::winit::dpi::LogicalSize;
use smithay::reexports::winit::platform::wayland::WindowAttributesExtWayland;
use smithay::reexports::winit::platform::x11::WindowAttributesExtX11;
use smithay::reexports::winit::window::Window as WinitWindow;
use smithay::utils::Transform;
use smithay::wayland::compositor::CompositorState;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::selection::data_device::DataDeviceState;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::XdgShellState;
use smithay::wayland::shm::ShmState;
use smithay::wayland::socket::ListeningSocketSource;
use smithay::wayland::viewporter::ViewporterState;

use state::{ClientState, State};

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("web: {msg}");
    std::process::exit(2)
}

fn main() {
    let start = Instant::now();
    let args = match args::parse(std::env::args().skip(1)) {
        Ok(Some(a)) => a,
        Ok(None) => {
            println!("{}", args::USAGE);
            return;
        }
        Err(e) => fail(format!("{e}\n{}", args::USAGE)),
    };
    let env = |k: &str| std::env::var(k).ok();
    let state_dir = host::state_dir(&args, &env).unwrap_or_else(|e| fail(e));
    // The engine needs an absolute profile path (CEF falls back to in-memory storage).
    let state_dir = std::path::absolute(&state_dir).unwrap_or_else(|e| fail(format!("{}: {e}", state_dir.display())));
    let profile = host::profile_dir(&state_dir, args.engine);
    create_private_dir(&profile).unwrap_or_else(|e| fail(format!("state directory {}: {e}", profile.display())));
    let exe = std::env::current_exe().ok();
    let host_bin = host::host_bin(&args, &env, exe.as_deref());
    // P-FORGET-SHELL: no window, no shell: forget the site and exit with the host's code.
    if let Some(site) = &args.forget_site {
        std::process::exit(forget::run_cli(site, &state_dir, &profile, host_bin, &args.host_args));
    }
    // C-SINGLE: a web already runs on this state directory: hand it the URL and leave.
    if let Some(code) = forget::run_second(args.url.as_deref(), &state_dir) {
        std::process::exit(code);
    }
    // C-SINGLE: the profile's lock (a stale one is removed, a live one is an error, no panic).
    match lock::check(&profile, std::path::Path::new("/proc")) {
        Ok(Some(line)) => eprintln!("[web] {line}"),
        Ok(None) => {}
        Err(m) => fail(m),
    }
    // C-SOCKDIR: run directories of shells that were killed hard.
    if let Some(runtime) = env("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        for d in forget::clean_stale_run_dirs(std::path::Path::new(&runtime)) {
            eprintln!("[web] removed the stale run directory {}", d.display());
        }
    }

    let mut event_loop: EventLoop<State> = EventLoop::try_new().unwrap_or_else(|e| fail(e));
    let display: Display<State> = Display::new().unwrap_or_else(|e| fail(e));
    let dh = display.handle();

    // The shell window on the user's desktop.
    let attrs = WinitWindow::default_attributes()
        .with_title("web")
        .with_inner_size(LogicalSize::new(1280.0, 800.0));
    let attrs = WindowAttributesExtWayland::with_name(attrs, "web", "web");
    let attrs = WindowAttributesExtX11::with_name(attrs, "web", "web");
    let (backend, winit_loop) =
        winit::init_from_attributes::<GlesRenderer>(attrs).unwrap_or_else(|e| fail(format!("cannot open the window: {e}")));
    let window_size = backend.window_size();
    // C6b: the desktop's scale for the window (winit may report the real one only later, as a Resized).
    let window_scale = backend.scale_factor();
    let desktop = {
        use smithay::reexports::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match backend.window().window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Wayland(_)) => "a Wayland",
            Ok(RawWindowHandle::Xlib(_)) | Ok(RawWindowHandle::Xcb(_)) => "an X11",
            _ => "an unknown",
        }
    };
    eprintln!(
        "[web +{}ms] the shell is {desktop} window on the desktop",
        start.elapsed().as_millis()
    );
    // C6: the desktop's clipboard, reached as the shell window.
    let desktop_clipboard = {
        use smithay::reexports::winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
        let kind = match backend.window().display_handle().map(|h| h.as_raw()) {
            Ok(RawDisplayHandle::Wayland(h)) => clipboard::DesktopKind::Wayland(h.display.as_ptr() as usize),
            Ok(RawDisplayHandle::Xlib(_)) | Ok(RawDisplayHandle::Xcb(_)) => clipboard::DesktopKind::X11,
            _ => clipboard::DesktopKind::None,
        };
        // C-KEYMAP: the desktop's keyboard layout goes to the seat (first keymap, then every change).
        let (keymap_tx, keymap_rx) = calloop::channel::channel::<keymap::Update>();
        let mut latest = keymap::Latest::default();
        event_loop
            .handle()
            .insert_source(keymap_rx, move |ev, _, state| match ev {
                calloop::channel::Event::Msg(keymap::Update::Keymap(text)) => {
                    if latest.is_new(&text) && !state.set_desktop_keymap(text) {
                        latest.forget();
                    }
                }
                calloop::channel::Event::Msg(keymap::Update::Group(g)) => state.set_desktop_group(g),
                calloop::channel::Event::Msg(keymap::Update::Mods(d, l, k)) => state.set_desktop_mods(d, l, k),
                calloop::channel::Event::Closed => {}
            })
            .unwrap();
        keymap::start(kind, keymap_tx);
        // C6e: the shell window's text input on the desktop (Wayland only).
        if let clipboard::DesktopKind::Wayland(display) = kind {
            let (ime_tx, ime_rx) = calloop::channel::channel::<ime::FromDesktop>();
            event_loop
                .handle()
                .insert_source(ime_rx, |ev, _, state| {
                    if let calloop::channel::Event::Msg(m) = ev {
                        state.ime_from_desktop(m);
                    }
                })
                .unwrap();
            ime::start(display, ime_tx);
        }
        clipboard::Desktop::start(kind)
    };
    let (copies_tx, copies_rx) = calloop::channel::channel::<selection::Copied>();
    event_loop
        .handle()
        .insert_source(copies_rx, |ev, _, state| {
            if let calloop::channel::Event::Msg(c) = ev {
                state.page_copied(c);
            }
        })
        .unwrap();

    // Our own Wayland socket for the engine host.
    let socket = ListeningSocketSource::new_auto().unwrap_or_else(|e| fail(format!("wayland socket: {e}")));
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    event_loop
        .handle()
        .insert_source(socket, |stream, _, state| {
            if let Err(e) = state.display_handle.insert_client(stream, Arc::new(ClientState::default())) {
                state.log(&format!("wayland client: {e}"));
            }
        })
        .unwrap();
    event_loop
        .handle()
        .insert_source(Generic::new(display, Interest::READ, CalloopMode::Level), |_, display, state| {
            // Safety: the display is not dropped while the loop runs.
            unsafe { display.get_mut().dispatch_clients(state).unwrap() };
            Ok(PostAction::Continue)
        })
        .unwrap();

    let output = Output::new(
        "web".into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "web".into(),
            model: "shell".into(),
        },
    );
    let _global = output.create_global::<State>(&dh);
    let mode = Mode {
        size: window_size,
        refresh: 60_000,
    };
    output.change_current_state(Some(mode), Some(Transform::Flipped180), None, Some((0, 0).into()));
    output.set_preferred(mode);

    let mut seat_state = SeatState::new();
    let mut seat = seat_state.new_wl_seat(&dh, "web");
    seat.add_keyboard(Default::default(), 400, 30)
        .unwrap_or_else(|e| fail(format!("keyboard: {e:?}")));
    seat.add_pointer();

    let mut space = Space::default();
    space.map_output(&output, (0, 0));

    let control = control::Control::listen().unwrap_or_else(|e| fail(format!("control socket: {e}")));
    let control_path = control.path.clone();
    if let Err(e) = forget::write_pid_file(&state_dir) {
        eprintln!(
            "web: cannot write {}: {e} (web --forget-site will not find this instance)",
            state_dir.join(forget::PID_FILE).display()
        );
    }

    let cmd = host::host_command(&args, host_bin, &profile, &control_path, &socket_name);
    eprintln!(
        "[web +{}ms] wayland socket {socket_name}, control {}, engine {} ({}), {}",
        start.elapsed().as_millis(),
        control_path.display(),
        args.engine.name(),
        cmd.program.display(),
        match &args.proxy {
            Some(p) => format!("proxy {p}"),
            None => "no --proxy: direct connections".into(),
        }
    );

    // `:help`'s page, and the channel external completion sources answer on (C3).
    let help_url = control::write_help(&control_path).unwrap_or_else(|e| {
        eprintln!("web: cannot write the help page: {e}");
        String::new()
    });
    let (complete_tx, complete_rx) = calloop::channel::channel::<completion::ExtResult>();
    let complete_tx = std::sync::Mutex::new(complete_tx);
    let sink: completion::ExtSink = Arc::new(move |r| {
        let _ = complete_tx.lock().unwrap_or_else(|e| e.into_inner()).send(r);
    });
    event_loop
        .handle()
        .insert_source(complete_rx, |ev, _, state| {
            if let calloop::channel::Event::Msg(r) = ev {
                state.keys.complete_result(r);
                state.redraw();
            }
        })
        .unwrap();

    // C6d: dmabuf when the shell's renderer can import them on a GPU, else wl_shm only.
    let mut backend = backend;
    let (dmabuf, dmabuf_line) = dmabuf::setup(backend.renderer(), &dh, std::env::var(dmabuf::DISABLE_ENV).ok().as_deref());
    eprintln!("[web +{}ms] {dmabuf_line}", start.elapsed().as_millis());
    // C-FLICKER: explicit sync on the dmabuf's device.
    // Without the guards (WEB_NO_BUFFER_SYNC) nothing would wait for an acquire point either.
    let buffer_sync = !dmabuf::disabled_by(std::env::var(sync::DISABLE_ENV).ok().as_deref());
    let (syncobj, syncobj_line) = syncobj::setup(
        &dh,
        dmabuf.device.as_deref(),
        !buffer_sync || dmabuf::disabled_by(std::env::var(syncobj::DISABLE_ENV).ok().as_deref()),
    );
    eprintln!("[web +{}ms] {syncobj_line}", start.elapsed().as_millis());
    let mut state = State {
        start,
        display_handle: dh.clone(),
        loop_signal: event_loop.get_signal(),
        backend,
        output,
        space,
        popups: PopupManager::default(),
        seat,
        compositor_state: CompositorState::new::<State>(&dh),
        xdg_shell_state: XdgShellState::new::<State>(&dh),
        _xdg_decoration_state: XdgDecorationState::new::<State>(&dh),
        shm_state: ShmState::new::<State>(&dh, vec![]),
        dmabuf,
        syncobj,
        held: sync::Held::default(),
        held_timer: false,
        buffer_sync,
        _output_manager_state: OutputManagerState::new_with_xdg_output::<State>(&dh),
        seat_state,
        data_device_state: DataDeviceState::new::<State>(&dh),
        primary_selection_state: PrimarySelectionState::new::<State>(&dh),
        bridge: selection::Bridge::new(desktop_clipboard.clone(), copies_tx),
        _viewporter_state: ViewporterState::new::<State>(&dh),
        _fractional_scale_state: smithay::wayland::fractional_scale::FractionalScaleManagerState::new::<State>(&dh),
        _cursor_shape_state: CursorShapeManagerState::new::<State>(&dh),
        _text_input_global: State::ime_global(&dh),
        // C6h: wp_presentation (CLOCK_MONOTONIC): Chromium's BeginFrame interval comes from its feedback.
        _presentation_state: smithay::wayland::presentation::PresentationState::new::<State>(&dh, libc::CLOCK_MONOTONIC as u32),
        presentation_clock: smithay::utils::Clock::new(),
        layout: layout::Layout::from_physical(window_size, window_scale),
        pairing: Default::default(),
        pending_toplevels: Vec::new(),
        page: None,
        pages: Vec::new(),
        shown: (0, None),
        page_size: (0, 0).into(),
        dialogs: Vec::new(),
        window_focused: true,
        pointer_loc: (0.0, 0.0).into(),
        cursor: CursorImageStatus::default_named(),
        cursor_log: String::new(),
        over_bar: false,
        url: String::new(),
        title: String::new(),
        dirty: true,
        dmabuf_seen: false,
        pacer: pacing::Pacer::new(pacing::DEFAULT_REFRESH_MHZ, desktop == "an X11"),
        pace_timer: false,
        frame_stats: std::env::var_os("WEB_FRAME_STATS").map(|_| (Instant::now(), 0)),
        probe: std::env::var(probe::ENV).ok().and_then(|v| probe::parse(&v)).map(|r| (r, 0)),
        control: None,
        forget_clients: Vec::new(),
        host: None,
        quit_deadline: None,
        exit_code: 0,
        bar: statusbar::Bar::new(),
        pick_overlay: picker::Overlay::new(),
        swallowed_button: None,
        click_mods_out: 0,
        keys: keys::Keys::new(Some(state_dir.clone()), help_url, Some(sink)),
        history: history::History::open(Some(state_dir.clone())),
        visits: Default::default(),
        last_click: None,
        bar_log: String::new(),
        damage_tracker: None,
        deferred: Default::default(),
        paused: false,
        loop_handle: event_loop.handle(),
        ime: Default::default(),
    };
    state.apply_output_state();
    state.damage_tracker = Some(OutputDamageTracker::from_output(&state.output));
    state.keys.clipboard = Some(Box::new(desktop_clipboard));

    control
        .register(&event_loop.handle())
        .unwrap_or_else(|e| fail(format!("control socket: {e}")));
    match cmd.spawn() {
        Ok(child) => {
            state.log(&format!("engine host pid {}", child.id()));
            state.host = Some(child);
        }
        Err(e) => fail(format!("cannot start the engine host {}: {e}", cmd.program.display())),
    }
    // After the spawn: Signals blocks these signals, and a child would inherit the mask.
    let signals = calloop::signals::Signals::new(&[calloop::signals::Signal::SIGTERM, calloop::signals::Signal::SIGINT])
        .unwrap_or_else(|e| fail(format!("signals: {e}")));
    event_loop
        .handle()
        .insert_source(signals, |ev, _, state| state.quit(&format!("{:?}", ev.signal())))
        .unwrap();

    event_loop
        .handle()
        .insert_source(winit_loop, |event, _, state| match event {
            WinitEvent::Resized { size, scale_factor } => state.resized(size, scale_factor),
            WinitEvent::Focus(focused) => {
                // C-FOCUSKEYS: winit's synthetic releases never reach us (smithay drops them): release
                // what the page holds while it still has the keyboard, and start clean on regaining it.
                state.release_pressed_keys();
                state.window_focused = focused;
                // Live checks wait for this before the first click (c6_live.py).
                state.log(&format!("shell window focused: {focused}"));
                if focused {
                    // C6: a copy made on the desktop meanwhile is offered to the page.
                    state.sync_from_desktop(clipboard::Which::Clipboard);
                    state.sync_from_desktop(clipboard::Which::Primary);
                }
                state.refocus();
                state.relayout();
            }
            WinitEvent::Input(event) => {
                state.process_input(event);
                state.redraw();
            }
            WinitEvent::Redraw => state.render(),
            WinitEvent::CloseRequested => state.quit("shell window closed"),
        })
        .unwrap();
    event_loop
        .handle()
        .insert_source(Timer::from_duration(Duration::from_millis(200)), |_, _, state| {
            state.watch_host();
            TimeoutAction::ToDuration(Duration::from_millis(200))
        })
        .unwrap();

    event_loop
        .run(None, &mut state, |state| {
            // C6c: a render wanted sooner than the outer frame spacing waits on a timer.
            if state.dirty && !state.pace_timer {
                match state.pacer.wait(Instant::now()) {
                    None => {
                        state.dirty = false;
                        state.backend.window().request_redraw();
                    }
                    Some(wait) => {
                        let armed = state.loop_handle.insert_source(Timer::from_duration(wait), |_, _, state| {
                            state.pace_timer = false;
                            state.dirty = false;
                            state.backend.window().request_redraw();
                            TimeoutAction::Drop
                        });
                        state.pace_timer = armed.is_ok();
                    }
                }
            }
            state.read_page_copies();
            state.space.refresh();
            state.popups.cleanup();
            let _ = state.display_handle.flush_clients();
        })
        .unwrap_or_else(|e| fail(e));

    let code = state.exit_code;
    if let Some(mut child) = state.host.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    drop(state);
    control::cleanup(&control_path);
    forget::remove_pid_file(&state_dir);
    std::process::exit(code);
}

fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}
