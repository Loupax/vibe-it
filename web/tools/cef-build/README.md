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
  means ThinLTO + CFI + PGO; a ThinLTO link of all of libcef is known to need far more memory
  and time than a plain link, and was not tried within the 20G cap (it would be a second
  multi-hour build to find out). A plain Release build (no LTO, no PGO) peaked at 12.3 GB
  with the compile jobs, not the link. Cost: somewhat slower JS/layout than the prebuilt (PGO
  and LTO are worth roughly 10% in Chromium's own numbers; not measured here). The last two
  args copy what CEF sets for its official Linux builds (no PartitionAlloc-as-malloc), so
  the allocator behaves like the prebuilt's.
- **Sysroot** (`use_sysroot=true`): Chromium's Debian bullseye sysroot, downloaded by the
  hooks, instead of Fedora's `-devel` packages (CEF's default for local builds is
  `use_sysroot=false`, which would need many of them). Nothing is installed on the system.
- **Variant A, not A2** (`web/notes/C-WEBEX.md`): OpenH264 and FFmpeg are Chromium's bundled
  copies. A2 (the system `libopenh264.so` and FFmpeg) is possible with Chromium's own
  unbundling (`build/linux/unbundle/openh264.gn`, `ffmpeg.gn`, `replace_gn_files.py`), but
  those libraries are not in the sysroot, so it means `use_sysroot=false` and building
  against Fedora's `-devel` packages (system packages this build does not install), and a
  system FFmpeg update could then break the engine. Not done; a separate item if wanted.

## Rebuild for a new CEF / Chromium security release

1. Bump the `cef`/`cef-dll-sys` crates in `web/host-cef/Cargo.toml` to the new version
   (cef-dll-sys refuses a distribution newer than the crate's version) and build the host
   once as usual (default `CEF_PATH`): that downloads the matching prebuilt, whose
   `include/cef_version.h` gives `CEF_COMMIT_HASH` (-> `CEF_COMMIT`) and
   `CHROME_VERSION_BUILD` (-> `CEF_BRANCH`).
2. `CEF_COMMIT=<hash> CEF_BRANCH=<build> web/tools/cef-build/build.sh` — sync moves the CEF
   checkout to the new commit and Chromium to the tag CEF names (shallow fetches, no
   history), re-applies CEF's patches and runs the hooks; the build is incremental (siso
   rebuilds what changed). Expected, not yet measured: a security release within one
   branch is a fraction of the first build (a version bump touches `chrome/VERSION`, which
   many targets depend on); a new branch (new Chromium major) is close to a fresh build.
3. Then `CEF_PATH=$HOME/.cache/web-cef-build/dist web/install.sh` as above, and the codec
   probe again.
4. Old `dist/<version>` dirs can be deleted by name once nothing uses them.

## Cost (first build, 2026-11-23/24, this machine: 16 threads, 31G, user working on it)

| | |
|---|---|
| Versions | CEF 156.0.31+ge603d97 (branch 8078, commit e603d97dd5cb8b0e37d8f1daa2bc5c7eab1aa7a5), Chromium 156.0.8078.100 (tag), depot_tools 132599ebb, Chromium's clang 24.0.0git (86922d51), Debian bullseye sysroot |
| sync | 37 min (2246 s; network-bound, ~29 GB) |
| build | 3 h 57 min (14206 s) at `-j 12`, nice 19, ~48 600 compile steps; plus a first 29 min run stopped to move siso's logs off /tmp (siso resumed where it stopped) |
| distrib + install | 45 s |
| Peak memory | 12.3 GB anon in the scope (sampled every 30 s); `MemoryPeak` with page cache 15.2 GB. The libcef link (`concurrent_links=1`, no LTO) is not the peak: 12 compile jobs of blink/V8 are. 20G cap never hit. |
| Disk | 40 GB in `~/.cache/web-cef-build`: Chromium checkout ~28.5 GB, `out/Release_GN_x64` 7.9 GB, depot_tools 1.2 GB, vpython 1.3 GB, `dist/` 0.6 GB, `cef/binary_distrib` 0.6 GB (a copy of dist; deletable), `tmp/` 0.5 GB (siso logs; deletable) |
| Result | `libcef.so` 533 MB (no debug info; the prebuilt is 1.48 GB with debug info); same file list as the prebuilt minimal distribution, same `cef_version.h` |
| No-op rebuild | `build.sh sync build`: 1 s each |

Everything except the codecs and the non-official flags is what the prebuilt has, and the
host's crate checks pass against it (`include/` is CEF's, API hash unchanged). Two generated
headers differ only in comments: `cef_api_versions.h` (dates, from `version_manager.py -a`
in sync) and `capi/cef_scheme_capi.h` (its `$hash=` line).

Outside `~/.cache/web-cef-build` the build leaves nothing: `TMPDIR` and
`VPYTHON_VIRTUALENV_ROOT` point inside it (siso's glog files and depot_tools' virtualenvs
otherwise land in /tmp and `~/.cache/vpython-root.<uid>`). depot_tools may create an empty
`~/.config/depot_tools`.

To free the space: `rm -rf ~/.cache/web-cef-build/src` (keeps `dist/`, so web keeps
building); the next build is then a fresh 4-5 hours.

## Checked (2026-11-24)

web-host-cef built against `dist/` (`CEF_PATH=~/.cache/web-cef-build/dist cargo build
--release`, cef-dll-sys: "Using versioned CEF path from environment: .../dist/156.0.31/...")
passes `host-cef/test/drive.py` in the headless nest (6G scope as usual), and the codec probe
in it reports: `RTCRtpSender` video H264 x6 (42001f/42e01f/4d001f, packetization 0/1),
`RTCRtpReceiver` H264 x8 (+ f4001f), offer rtpmaps with H264, `canPlayType` avc1.42E01E and
mp4a.40.2 "probably", MSE avc1 true, MediaRecorder h264/avc1 true, WebCodecs avc1
encode/decode true (the prebuilt: none of these; `web/notes/C-WEBEX.md`). HEVC stays "".
