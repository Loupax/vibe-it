# C-WEBEX: why Webex says "Your browser doesn't support shared content and video"

Report for web/TODO.txt C-WEBEX (2026-11-23). Investigation only: no code, switch or UA was
changed. All runs: the installed web (`~/.local/lib/web/current`, main 2d48e312, CEF
156.0.31+chromium-156.0.8078.100, the "minimal" Spotify-CDN distribution), headless nested dwl
(web/tools/nest.sh), every CEF/Chrome process in a 6G memory-capped scope, throwaway profiles
under `.qwork/`. No Webex account was used and no meeting was joined: Webex's web app was only
downloaded as static files (`https://web.webex.com/` and its `/scripts/*.js`, no login).

## Answer in short

**Both messages come from one check: "can this browser receive H.264 over WebRTC?". web's CEF
build has no H.264 at all, so the check fails.** Nothing else Webex reads differs in a way it
cares about: not the User-Agent, not the client-hint brands, not `getDisplayMedia`.

- Webex's web app (Webex's own bundle, also the open-source SDK it embeds) computes
  `isCapableOfReceivingH264Codec = ["video/H264","video/h264"].some(m =>
  WebCapabilities.isCapableOfReceivingVideoCodec(m) === CAPABLE)`, where
  `isCapableOfReceivingVideoCodec(m)` is `RTCRtpReceiver.getCapabilities('video').codecs.some(c =>
  c.mimeType === m)`. The banner component `H264CodecMissingBanner` is shown when that is false;
  its message id `H264CodecMissingBanner.missingH264Codec` is "Your browser doesn't support shared
  content and video. For best experience, use another supported browser."
- The join-time message `JoinCall.missingH264Codec` ("Your browser needs the Cisco OpenH264 Video
  Codec to join. Install or turn on the codec, then try again.") belongs to the same codec state.
  The SDK side (`@webex/plugin-meetings`, `MeetingsUtil.hasH264Codec` / `checkH264Support`, public
  in github.com/webex/webex-js-sdk at d90e1921, and in Webex's bundled `webex-*.js`) first asks
  the same `isCapableOfReceivingVideoCodec('video/H264')` and, only if that is UNKNOWN, makes an
  `RTCPeerConnection` offer and looks for `/^a=rtpmap:\d+\s+H264\/\d+/m`; when missing it fires
  `media:codec:missing` and re-checks every 5 s for 5 minutes. The wording is aimed at Firefox,
  which downloads Cisco's OpenH264 plugin on demand; Chromium has no such download.
- In web-host-cef both inputs say "no H.264" (probe below): `RTCRtpReceiver.getCapabilities`
  and `RTCRtpSender.getCapabilities` list VP8, VP9, AV1 only, and the offer SDP has no H264
  rtpmap line. Stock Google Chrome 156 on the same machine lists six (send) / eight (receive)
  H.264 profiles and offers them.
- Why: CEF's binary distribution is built without `proprietary_codecs`, and WebRTC's
  `rtc_use_h264` (Chromium's built-in OpenH264 encoder + FFmpeg H.264 decoder) is only on when
  `proprietary_codecs` is. The same build gap is C-H264 (parked): `canPlayType avc1` is "",
  MSE, MediaRecorder and WebCodecs have no H.264 either. `strings libcef.so` finds no OpenH264
  in web's libcef; Fedora's own `cef` package (below) has it.
- Webex needs H.264 for real, not just in a check: its media servers send and expect H.264
  video and content share. Pretending (patching `getCapabilities` from a page script) would
  hide the banner but negotiation would fail; no item proposed for that.

## Evidence

### Capability probe: web-host-cef vs. stock Chrome

A local test page (`web/tools/codec-probe/probe.html`, served by `serve.py` next to it, which saves
what the page POSTs) read the values below. Chrome: `google-chrome-stable 156.0.8078.47 --headless=new`, own profile
under `.qwork/`, background traffic switched off. web: the installed host through
`web/host-cef/run.sh` in the headless nest, and the same page through the `web` shell (identical
results). Only the rows that differ are listed; everything else matched (getUserMedia and
getDisplayMedia present, `enumerateDevices`, camera permission "prompt", `window.chrome`,
`navigator.vendor` "Google Inc.", insertable streams, `MediaStreamTrackProcessor`, CropTarget,
RestrictionTarget, capture handles, WebGL2, WebGPU object, AV1/VP8/VP9 in WebCodecs).

| What the page read | Chrome 156 | web-host-cef (CEF 156) |
|---|---|---|
| `RTCRtpSender.getCapabilities('video')` | VP8, H264 x6 (42001f, 42e01f, 4d001f; pm 0/1), AV1, VP9 x2 | VP8, AV1, VP9 x2 |
| `RTCRtpReceiver.getCapabilities('video')` | ... H264 x8 (also f4001f) ... | VP8, VP9 x4, AV1 x2 (no H264) |
| `createOffer` rtpmap (video) | VP8, H264 x6, AV1, VP9 x2 | VP8, AV1, VP9 x2 |
| `mediaCapabilities.encodingInfo/decodingInfo` webrtc `video/H264` | supported | not supported |
| `VideoEncoder/VideoDecoder.isConfigSupported` `avc1.42001f` | true / true | false / false |
| `MediaRecorder.isTypeSupported` webm;h264, mp4;avc1 | true | false |
| `canPlayType` mp4 avc1 / mp4a.40.2 (AAC) | "probably" / "probably" | "" / "" |
| `MediaSource.isTypeSupported` mp4 avc1 | true | false |
| `navigator.userAgentData.brands` | Not:A-Brand 8, Chromium 156, **Google Chrome 156** | Not:A-Brand 8, Chromium 156 |
| `navigator.userAgent` | `... HeadlessChrome/156.0.0.0 ...` (headless only) | `... Chrome/156.0.0.0 Safari/537.36` |

### What Webex's bundle checks (static reading)

From `https://web.webex.com/scripts/*.js` (downloaded 2026-11-23, 28 files, 11 MB; not committed):
- `68535-*.js`: messages `H264CodecMissingBanner.missingH264Codec` (the banner text) and the hook
  `t = !a.isCapableOfReceivingH264Codec` that shows it.
- `623411-*.js`: `isCapableOfReceivingH264Codec` = the `RTCRtpReceiver.getCapabilities` test above.
- `webex-*.js`: the SDK's `hasH264Codec` (receiver capabilities, then the SDP regex) and
  `media:codec:missing` / `media:codec:loaded`.
- `257774-*.js`: `JoinCall.missingH264Codec` (the join text); the code that raises it is in a
  chunk loaded later (not downloaded); it sits next to the other join errors.
- No brand check: `userAgentData` is read only by the bundled UA parser; there is no
  `"Google Chrome"` string. A separate version check (`browserSupportDecision`, message
  `unsupportedBrowserVersion`, an "old browser" banner) exists; it is not what the user saw and
  `Chrome/156` passes it.

### Screen sharing (`getDisplayMedia`) works up to the OS portal

- A click on the probe page's button calls `getDisplayMedia({video:true})`. In web-host-cef,
  and through the `web` shell, Chrome's own picker appears ("Choose what to share with
  http://127.0.0.1:18731", tabs "Chromium Tab", "Window", "Entire Screen"; the shell shows it as
  a dialog over the page). CEF's Chrome style brings this picker with it; the host needs no media
  handler for it.
- Choosing "Window" or "Entire Screen" starts WebRTC's PipeWire capturer, which asks
  xdg-desktop-portal for a ScreenCast session: the log has
  `desktop_capture/linux/wayland/screen_capture_portal_interface.cc ... Failed to request session`
  and `base_capturer_pipewire.cc ... ScreenCastPortal failed` (the run used a private D-Bus with
  no portal, so the user's real screen could not be offered). That is the same code path as
  Chrome on Wayland; the user's session has xdg-desktop-portal-wlr configured for ScreenCast
  (`/usr/share/xdg-desktop-portal/wlroots-portals.conf`), so in the real session it should behave
  as Chrome does.
- Not verified end to end: with a private bus that starts only xdg-desktop-portal and
  xdg-desktop-portal-wlr against the nest, portal-wlr 0.8.4 exited with "Compositor supports
  neither ext_image_copy_capture or wlr_screencopy!" although grim works in the same nest; I did
  not chase it (it is a test-setup question, not a web one).
- The "Chromium Tab" list is empty: web's other buffers are not offered for tab sharing (each
  page is its own window, not a tab in Chrome's tab strip). Minor; sharing a window/screen works
  the portal way.
- For Webex this does not matter yet: without H.264 Webex disables content share anyway.

## What each fix costs

| Option | What changes | Cost | Touches |
|---|---|---|---|
| A. Build CEF 156 ourselves with `proprietary_codecs=true ffmpeg_branding=Chrome` (WebRTC's `rtc_use_h264` follows) | `CEF_PATH` points at our own build instead of the Spotify-CDN minimal one; host code unchanged | Chromium checkout + build: on the order of 100+ GB disk and hours of CPU per build, and linking libcef needs far more than the 6G cap (user approval needed for a bigger cap, or a build machine); rebuilt for every CEF/Chromium security release (every 1-2 weeks) or web falls behind. Binary grows by a few MB (FFmpeg H.264/AAC decoders + OpenH264 encoder). Licensing: H.264/AAC are patent-encumbered; Cisco pays the OpenH264 royalties only for Cisco's own prebuilt binaries, not for OpenH264 compiled into Chromium; a private build for the user's own use is the usual grey zone, but the binary must never be published (the repo is public). No Google traffic, no privacy change. | C-H264 (fixes it whole: YouTube-style avc1, AAC, MSE, WebCodecs, Webex) |
| A2. Same, but with Fedora's patches that link the system `libopenh264.so` (Cisco's binary from the `fedora-cisco-openh264` repo, royalty covered) and the system FFmpeg (`libavcodec-freeworld` from RPM Fusion is installed here) | as A, plus carrying Fedora's Chromium patches | as A, plus patch upkeep per release; licence exposure smaller (the codecs are the distro's) | C-H264 |
| B. Use Fedora's `cef` package (`/usr/lib64/cef`, already installed for OBS: CEF 146.0.11, Chromium 146.0.7680.177) | host built against CEF 146 (cef crate 146.x, API port back from 156) | No build. `strings` shows it links `libopenh264.so.8` and the system `libavcodec` (OpenH264 encoder present). **But it is ten Chromium majors behind** (last Fedora update 2026-04-15, CVEs since unpatched) and moves only when OBS needs it; web's host would have to follow its API version. Not recommended as the default engine. | C-H264 |
| C. Third-party prebuilt CEF with proprietary codecs | `CEF_PATH` | Trust in an unknown builder for the whole browser engine; versions lag; not checked. Not recommended. | C-H264 |
| D. Do nothing in the engine; open such sites elsewhere | a command that hands the current URL to another browser (e.g. `xdg-open`, or a configured one) | Tiny. The user keeps using Chrome or the Webex desktop app for Webex. | none |
| E. UA / client hints claiming "Google Chrome" | `userAgentData` brands | Not needed for Webex (no brand check). Spoofing; skip. | none |

## Proposed TODO items (for the coordinator to file)

- **C-H264** (parked by the user; add to its text): "Webex needs it (C-WEBEX): its web app tests
  `RTCRtpReceiver.getCapabilities('video')` for `video/H264` and refuses video, content share and
  joining without it. Options and costs: web/notes/C-WEBEX.md (own proprietary-codecs CEF build
  A/A2, Fedora's cef 146 B, external browser D)." The user decides whether it stays parked.
- **C-H264-BUILD [L]** (only if the user un-parks C-H264 and picks A or A2): build CEF 156 from
  source with `proprietary_codecs=true ffmpeg_branding=Chrome` (A2: plus system openh264/ffmpeg
  patches) into `$HOME/.cache/`, never published; install.sh takes it via `CEF_PATH`; a script
  that repeats the build for a new CEF version. Needs the user's OK for the disk, build time and a
  memory cap above 6G for the link. Done when the C-WEBEX probe lists H264 in both
  `RTCRtpSender`/`RTCRtpReceiver` capabilities and `canPlayType('video/mp4; codecs="avc1.42E01E"')`
  is "probably", and the user sees no H.264 banner on Webex.
- **C-EXTERNAL [S]** (option D, independent of C-H264): `:openExternal [url]` hands the current (or
  given) URL to another browser (`$BROWSER` or `xdg-open`; configurable), so a site web cannot
  serve (Webex now, DRM sites per C-DRM) is one command away. Done when a live check sees the
  command run with the page's URL (a stub browser in the test).
- **C-SHARE-CHECK [S]** (user, 2-minute check on the real desktop; no code): in web, open a
  getDisplayMedia test page (e.g. https://webrtc.github.io/samples/src/content/getusermedia/getdisplaymedia/),
  start sharing, pick "Entire Screen" or "Window" in Chrome's picker, confirm the portal's
  chooser, and see the shared picture. If that fails where Chrome works, file a fix item with the
  host log lines (`base_capturer_pipewire`, `screen_capture_portal_interface`).
- *(optional, low)* **C-SHARE-TAB [S]**: Chrome's picker lists no "Chromium Tab" entries, so web's
  buffers cannot be shared as tabs. Find out whether CEF can list its own Chrome-style browsers
  there; else document.

## Small fixes

None made. The host switches (`protocol::quiet_switches`, C-BGNET) do not affect any of this:
the H.264 gap is in the build, and the C-BGNET switches stop only Google's background traffic
(the original C-BGNET probe already found H.264 missing with and without them).

## Reproducing

    web/tools/nest.sh                                          # headless dwl, e.g. wayland-2
    python3 -I web/tools/codec-probe/serve.py 18731 .qwork/probe/out &   # results: .qwork/probe/out/<tag>-caps.json
    WAYLAND_DISPLAY=wayland-2 WEB_HOST_BIN=$HOME/.local/lib/web/current/web-host-cef \
      WEB_HOST_PROFILE=$PWD/.qwork/cef-profile timeout 30 web/host-cef/run.sh \
      --url 'http://127.0.0.1:18731/probe.html?tag=cef'
    # Chrome: google-chrome-stable --headless=new --user-data-dir=$PWD/.qwork/chrome-profile \
    #   --disable-background-networking --component-updater=url-source=http://127.0.0.1:9/ URL
    # getDisplayMedia: run the host under `dbus-run-session --config-file=<bus with no services>`
    # (so no portal can offer the real screen), click the page with web/tools/inject, grim the nest.

The probe page reads: `navigator.userAgent`, `userAgentData` (+ high-entropy values),
`RTCRtpSender/Receiver.getCapabilities('video'|'audio')`, the video rtpmap lines of a
`createOffer`, `MediaRecorder.isTypeSupported`, `canPlayType`, `MediaSource.isTypeSupported`,
WebCodecs `isConfigSupported` (avc1, vp8, vp9, av1), `mediaCapabilities` webrtc H.264,
`getDisplayMedia`/`getUserMedia` presence, `enumerateDevices`, camera permission state, and a
`getDisplayMedia` call on a click.
