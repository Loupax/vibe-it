# web spike

Throwaway spike code for `web/TODO.txt` (W1..W7). Its own workspace root: it joins
no other build and is not in CI.

W1 state: one CEF window (Views, Chrome runtime style) loads one URL. No request
interception, no network namespace yet (direct network).

## Versions

- `cef` crate `=156.7.0+156.0.31` (tauri-apps/cef-rs), `default-features = false`
- CEF `156.0.31+ge603d97`, Chromium `156.0.8078.100`, the `linux64_minimal` archive
  (downloaded by `cef-dll-sys`'s build script)
- Build needs cmake and ninja (for CEF's `libcef_dll_wrapper`) and a C++ compiler.

## Fetch CEF and build

CEF lives in a cache outside the repo. `CEF_PATH` tells `cef-dll-sys` where; on the
first build it downloads the archive (~380 MB .tar.bz2, ~1.9 GB with the extracted
tree) to `$CEF_PATH/156.0.31/cef_linux_x86_64/`:

    cd web/spike
    export CEF_PATH=$HOME/.cache/web-spike/cef
    CARGO_BUILD_JOBS=4 nice -n 19 ionice -c3 cargo build

`target/` is ~2 GB afterwards (libcef.so is copied in). Delete it when done:
`rm -rf web/spike/target`. Delete the CEF cache with `rm -rf $HOME/.cache/web-spike`.

build.rs works around two things in `cef-dll-sys` 156.7.0:
- it copies libcef.so and its resources to `OUT_DIR/../../..`, which with the
  current cargo build-dir layout (`target/debug/build/<crate>/<hash>/out`) is
  `target/debug/build/`, not the binary's directory. build.rs hard-links them from
  the CEF dir (`DEP_CEF_DLL_WRAPPER_CEF_DIR`; `cef-dll-sys` is a direct dependency
  for that, which also orders the build scripts) to `target/debug/`.
- it sets no rpath; build.rs adds `$ORIGIN`.

Unit test (URL normalisation only): `cargo test` (same `CEF_PATH`).

## Run

    web/spike/run.sh [--x11] [URL] [extra Chromium switches]

- Runs the binary in `systemd-run --user --scope -p MemoryMax=6G -p MemorySwapMax=0`
  as unit `$WEB_SPIKE_UNIT` (default `web-spike-<pid>`). Stop the whole process tree
  with `systemctl --user stop <unit>.scope`.
- Profile/cache: `$WEB_SPIKE_PROFILE` (default `web/spike/.profile/`, git-ignored).
  The binary refuses to start without it, so CEF never falls back to `~/.config`.
  CEF's own log goes to `<profile>/cef.log`; the spike's lines go to stderr as
  `[web-spike +<ms>ms] ...` (title, main-frame load end with HTTP status, load
  errors, console messages).
- Wayland by default (`--ozone-platform=wayland`, needs `WAYLAND_DISPLAY`);
  `--x11` passes `--ozone-platform=x11` (needs `DISPLAY`).

In the nested compositor (never the user's display):

    scratchy/tools/nest.sh                    # prints e.g. wayland-1
    env -u DISPLAY WAYLAND_DISPLAY=wayland-1 WEB_SPIKE_UNIT=web-spike-w1 web/spike/run.sh https://example.com/
    # X11: the nested dwl has Xwayland; its display is the X socket dwl listens on
    # (`ss -xlp | grep X11-unix` shows dwl's PID next to it, e.g. :1)
    env -u WAYLAND_DISPLAY DISPLAY=:1 WEB_SPIKE_UNIT=web-spike-w1 web/spike/run.sh --x11 https://example.com/
    systemctl --user stop web-spike-w1.scope
    scratchy/tools/nest.sh --stop

## What W1 learned (for W2..W7)

- **Process model.** One binary for everything: CEF re-executes it with
  `--type=zygote|gpu-process|utility|renderer` and `execute_process` runs that
  helper. A page load gives ~10 processes: browser, two zygotes (sandboxed and
  `--no-zygote-sandbox`), GPU, the network service
  (`--type=utility --utility-sub-type=network.mojom.NetworkService
  --service-sandbox-type=none`), storage/audio utilities, renderer(s).
- **Sandbox.** Chromium's own Linux sandbox works unprivileged (user namespaces;
  no SUID `chrome-sandbox` needed): renderers and sandboxed utilities already sit in
  their own user, pid **and network** namespace. The network service, browser and
  GPU process do not; they are what W2's `unshare -rn` has to cover.
  `WEB_SPIKE_NO_SANDBOX=1` sets `no_sandbox` if nested namespaces under `unshare -rn`
  turn out to be a problem.
- **Chromium leaves the memory-capped scope unless told not to.** At startup its
  xdg-portal code (`components/dbus/xdg/systemd.cc`) asks systemd over D-Bus
  (`StartTransientUnit`) to move the browser process into its own
  `app-org.chromium.Chromium-<pid>.scope`. The browser process then escapes the
  `MemoryMax` cap and `systemctl --user stop <unit>` no longer stops it. It skips
  this when `FLATPAK_SANDBOX_DIR` or `SNAP` is set, so run.sh sets
  `FLATPAK_SANDBOX_DIR` (nothing else in libcef reads it). Naming our scope `app-*`
  does not help. Keep this for every later run script; the session D-Bus socket is a
  path, so it is still reachable inside `unshare -rn`.
- **Startup** (debug build, warm disk cache): `cef::initialize` to
  `on_context_initialized` ~100-270 ms; example.com main frame loaded at ~200-570 ms,
  en.wikipedia.org at ~600-750 ms after process start.
- **Memory** (Wikipedia main page, one tab): process RSS sum ~1.05-1.2 GB (shared
  libcef pages counted per process: browser ~245 MB, GPU ~235 MB, renderer
  ~230 MB, network service ~110 MB, zygotes 20-75 MB each); the scope's own
  `MemoryCurrent` ~300 MB.
- **Wayland noise:** `'--ozone-platform=wayland' is not compatible with Vulkan` and
  one `ContextResult::kTransientFailure` from the GPU process on start; harmless,
  the page renders.
