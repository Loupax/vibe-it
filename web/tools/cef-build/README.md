# cef-build: CEF with proprietary codecs (C-H264-BUILD)

`build.sh` builds the CEF binary distribution web-host-cef uses (CEF 156.0.31, the version
the `cef` crate `=156.7.0` pins) from source with H.264/AAC turned on, so `<video>`/MSE play
avc1 and WebRTC offers H.264 (OpenH264 encoder, FFmpeg decoder): what Webex checks
(`web/notes/C-WEBEX.md`). The user approved it (`web/DECISIONS.md` 2026-11-23 "H.264: build
CEF ourselves").

**Never commit, upload or publish the result** (H.264/AAC are patent-encumbered; this
repository is public). Only this script and its docs live in the repo; the checkout, the
build and the distribution stay in `~/.cache/web-cef-build`.

## Use it

    web/tools/cef-build/build.sh          # all steps; hours; see "Cost" below
    CEF_PATH=$HOME/.cache/web-cef-build/dist web/install.sh
    # or for a dev build of the host:
    cd web/host-cef && CEF_PATH=$HOME/.cache/web-cef-build/dist cargo build --release

`install.sh` and the host's build take the distribution through `CEF_PATH` as they take the
prebuilt one (`$HOME/.cache/web-spike/cef` stays the default): `build.sh install` lays it out
as `dist/156.0.31/cef_linux_x86_64/` with an `archive.json`, which is what cef-dll-sys looks
for. cef-dll-sys re-runs when `CEF_PATH` changes, so switching back and forth just rebuilds.
Check the result with the codec probe (`web/tools/codec-probe/`): `canPlayType('video/mp4;
codecs="avc1.42E01E"')` is `"probably"` and `RTCRtpSender/RTCRtpReceiver.getCapabilities('video')`
list `video/H264`.

## Steps

`build.sh [sync|gen|build|distrib|install|clean-out|all]`, each re-runnable on its own, each
run as `systemd-run --user --scope -p MemoryMax=$MEM -p MemorySwapMax=0 nice -n 19 ionice -c3`
(scope unit `web-cef-build-<step>-<pid>`, stop it with `systemctl --user stop <unit>.scope`),
logging to `~/.cache/web-cef-build/logs/<step>-<time>.log`.

| Step | What |
|---|---|
| sync | CEF's `automate-git.py` (fetched from the pinned CEF commit) with `--no-chromium-history --no-build --no-distrib`: depot_tools, CEF at `CEF_COMMIT`, Chromium at the version CEF names (`CHROMIUM_BUILD_COMPATIBILITY.txt`), CEF's patches, the hooks (clang, the Debian sysroot, ...). depot_tools is frozen after the first clone (`--no-depot-tools-update`). |
| gen | `cef/tools/gclient_hook.py` with `GN_DEFINES` and `GN_OUT_CONFIGS=Release_GN_x64` (Release only, no Debug config). |
| build | `autoninja -j $JOBS -C out/Release_GN_x64 cefsimple chrome_sandbox` (cefsimple pulls in libcef.so and all of its runtime files; it is the smallest target that does). |
| distrib | `make_distrib.py --minimal --no-symbols --no-docs --no-archive` |
| install | copy into `dist/<cef version>/cef_linux_x86_64` (+ `archive.json`) |

Env: `CEF_BUILD_ROOT` (default `~/.cache/web-cef-build`), `CEF_BRANCH` (8078), `CEF_COMMIT`
(e603d97dd5cb, = 156.0.31), `JOBS` (12), `LINKS` (1), `MEM` (20G), `EXTRA_GN_DEFINES`.

## Choices

- `GN_DEFINES`: `is_official_build=false is_debug=false dcheck_always_on=false symbol_level=0
  blink_symbol_level=0 v8_symbol_level=0 proprietary_codecs=true ffmpeg_branding="Chrome"
  rtc_use_h264=true use_sysroot=true concurrent_links=1 disable_fieldtrial_testing_config=true
  use_partition_alloc_as_malloc=false enable_backup_ref_ptr_support=false`.
- **Not an official build.** Spotify's prebuilt is `is_official_build=true`, which on Linux
  means ThinLTO + CFI + PGO; a ThinLTO link of libcef needs well over the 20G cap the user
  allowed for this build. A plain Release build (`-O2`, no LTO, no PGO) links in a few GB.
  Cost: somewhat slower JS/layout than the prebuilt (Chromium quotes ~10% for PGO and
  LTO together). The last two args copy what CEF sets for its official Linux builds (no
  PartitionAlloc-as-malloc), so the allocator behaves like the prebuilt's.
- **Sysroot** (`use_sysroot=true`): Chromium's Debian bullseye sysroot, downloaded by the
  hooks, instead of Fedora's `-devel` packages (CEF's default for local builds is
  `use_sysroot=false`, which would need many of them). Nothing is installed on the system.
- **Variant A, not A2** (`web/notes/C-WEBEX.md`): OpenH264 and FFmpeg are Chromium's bundled
  copies. A2 (the system `libopenh264.so` and FFmpeg via Fedora's patches) cannot be combined
  with the sysroot (those libraries are not in it) and means carrying Fedora's patches per
  release.

## Rebuild for a new CEF / Chromium security release

1. Bump the `cef`/`cef-dll-sys` crates in `web/host-cef/Cargo.toml` to the new version
   (cef-dll-sys refuses a distribution newer than the crate's version), and look up the
   matching CEF commit: the crate's version says it (`156.x.y+156.0.Z`), and
   `git ls-remote https://github.com/chromiumembedded/cef.git refs/heads/<branch>` or the
   tag/commit in CEF's release notes gives `CEF_COMMIT` for `156.0.Z`.
2. `CEF_COMMIT=<new> web/tools/cef-build/build.sh` — sync updates CEF and Chromium in place
   (fetching only the new commits; no history), and ninja rebuilds only what changed (a
   security release touches a handful of files, but a Chromium version bump touches
   `chrome/VERSION`, which recompiles a fair share).
3. A new Chromium major (new branch): also set `CEF_BRANCH`. That is close to a fresh build.
4. Old `dist/<version>` dirs can be deleted by name once nothing uses them.

## Cost (measured on this machine, 16 threads, 31G, `-j 12`)

See the numbers in the C-H264-BUILD PR / `web/PROMPTS.md`; filled in below after the first build.
