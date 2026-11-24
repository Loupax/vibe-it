# C-BELL-SPIKE: can the host see a page's Web Notifications?

Report for web/TODO.txt C-BELL step 1 (2026-11-24). Spike only: nothing is built for the shell.

Setup: web-host-cef at origin/main 901e6a1c (CEF 156.0.31, cef crate 156.7.0, debug build),
headless nested dwl (web/tools/nest.sh), every host in its 6G scope with a throwaway profile under
`.qwork/`. Test page `web/tools/notify-probe/probe.html` served from 127.0.0.1 (a secure context).
**Every run is on a private D-Bus session bus** (`web/tools/notify-probe/run.sh`: `dbus-run-session`
with `private-bus.conf`, which has no service activation) where `fake_notifyd.py` owns
`org.freedesktop.Notifications` and logs every call, so nothing could reach the user's desktop
notifications and the log shows whether Chromium tries.

    web/tools/nest.sh                                        # e.g. wayland-2
    WAYLAND_DISPLAY=wayland-2 web/tools/notify-probe/run.sh .qwork/np TAG 16
    WEB_HOST_NOTIFY_SPIKE=log|grant|shim WAYLAND_DISPLAY=... web/tools/notify-probe/run.sh ...

The page asks `Notification.permission`, `navigator.permissions.query({name:'notifications'})`,
`Notification.requestPermission()` (8 s timeout), `new Notification(...)`, registers an empty service
worker and calls `registration.showNotification(...)` and `getNotifications()`, then takes
`Notification` from a fresh about:blank iframe and constructs one (a page could fetch a pristine API
that way). Results: `OUT/TAG.json`, `TAG.host.log`, `TAG.notifyd.log`, a mid-run screenshot.

## Answers in short

1. **Runtime: Chrome style** (host-cef README "Chrome runtime style, no Chrome UI"; Chrome-style
   BrowserViews, `CEF_CTT_NONE`). Not Alloy.
2. **Today (no handler):** `Notification.permission` = `"default"`, `permissions.query` = `"prompt"`,
   `requestPermission()` **never settles** (still pending after 8 s): Chrome draws its own permission
   bubble ("http://127.0.0.1:PORT wants to Show notifications [Block] [Allow]") over the page as an
   xdg popup, and nothing answers it. `new Notification` fires `error`; `showNotification` rejects
   ("No notification permission has been granted for this origin"). Nothing reaches D-Bus but
   Chromium's startup `GetCapabilities` + `GetServerInformation` (it probes the notification server
   at start in every run, also on the user's bus today: harmless, nothing pops).
   **If the user clicked Allow on that bubble today, the page's notifications WOULD pop up on the
   desktop** (see 3): that is the current behaviour, not something this spike added.
3. **CEF hook: only the permission prompt, never the notification.** `CefPermissionHandler::
   OnShowPermissionPrompt` is called (`permissions 0x8000` = `CEF_PERMISSION_TYPE_NOTIFICATIONS`,
   with the origin). Accepting it (`WEB_HOST_NOTIFY_SPIKE=grant`) makes everything work for the page
   (`granted`, `show` event, `showNotification` resolves, `getNotifications` lists it) and **Chrome
   shows the notification itself through its Linux platform bridge on the session bus**:

       Notify app='Chromium' replaces=0 summary='probe title'
              body='<a href="http://127.0.0.1:21141/">127.0.0.1:21141</a>\n\nprobe body'
              actions=['default', 'Activate', 'settings', 'Settings'] hints=['desktop-entry', 'urgency']
       Notify app='Chromium' replaces=0 summary='sw title' body='<a href=...>\n\nsw body' ...

   CEF 156 has no handler or callback for the notification itself (no `notification` anywhere in
   `include/`: only the permission type and content setting). The D-Bus call is the only place it
   surfaces: no tab, no tag (Chrome keeps the tag and uses `replaces_id`), the origin only as markup in
   the body. Intercepting it would mean giving Chromium a session bus of our own (or a filtering
   proxy) that we serve `org.freedesktop.Notifications` on, which also cuts it off from the portals
   (file chooser, screen sharing via xdg-desktop-portal + PipeWire) and anything else on the real bus.
   **Not viable.** With no notification server at all Chromium falls back to its own message-center
   popups (not checked; they would be Chrome UI windows, also not wanted).
4. **The fallback works: a renderer shim** (`WEB_HOST_NOTIFY_SPIKE=shim`). In
   `RenderProcessHandler::OnContextCreated` (every frame, every context) the renderer evals a small
   script with a native V8 function as its argument (`v8_value_create_function`; nothing is left on
   the global object). The script replaces `Notification` (permission `"granted"`,
   `requestPermission()` resolves `"granted"`, the constructor posts and fires `show`),
   `ServiceWorkerRegistration.prototype.showNotification` / `getNotifications`, and
   `permissions.query({name:'notifications'})`. The native function sends a CEF process message
   (`web-host.notify`: title, body, tag, kind, frame URL, is-main) to the browser process, which
   knows the tab from the browser. Host log of one run (`shim2`):

       notify-spike: shim installed in 629 us (renderer pid 1)
       notify-spike: NOTIFY tab Some(1) kind page title "probe title" body "probe body" tag "probe-tag" frame http://127.0.0.1:25501/probe.html?tag=shim2 (main true)
       notify-spike: NOTIFY tab Some(1) kind sw title "sw title" body "sw body" tag "sw-tag" frame http://127.0.0.1:25501/probe.html?tag=shim2 (main true)
       notify-spike: shim installed in 162 us (renderer pid 1)
       notify-spike: NOTIFY tab Some(1) kind page title "iframe title" body "iframe body" tag "iframe-tag" frame about:blank (main false)

   Page side: `{"permission_before":"granted","query_before":"granted","request_result":"granted",
   "ctor":"show","sw_show":"resolved","sw_get":[],"iframe_permission":"granted","iframe_ctor":"show"}`.
   The fake notification server saw no `Notify` and Chrome showed no permission bubble (the page never
   reaches Chrome's real `requestPermission`). Off (no env): the same results as today, no
   `notify-spike` line.

## The trap found on the way: secure-context APIs arrive after OnContextCreated

The first shim (plain assignment of `Notification` and of `ServiceWorkerRegistration.prototype
.showNotification`) changed only `permissions.query`; the page still saw Chrome's own `Notification`.
Probe inside the shim: at OnContextCreated `typeof Notification` and `typeof
ServiceWorkerRegistration` are both `"undefined"` (`isSecureContext` already true); in a microtask
queued from there `Notification` exists and `ServiceWorkerRegistration` still does not; by
`setTimeout 0` both exist. Blink installs `[SecureContext]` interfaces after the context-created
hook and they overwrite what we put there. What works:

- `Notification`: define ours on the global first as a **non-configurable, non-writable** property;
  Blink's later define of the real interface object then fails silently (no crash, no CHECK, page
  and iframe both get ours).
- `ServiceWorkerRegistration`: a window page can only reach a registration through
  `navigator.serviceWorker`, so the shim puts an own getter `serviceWorker` on that `navigator`
  object that patches `ServiceWorkerRegistration.prototype` on first use and returns the real
  container (`Navigator.prototype`'s getter).

## What it costs

- Code: ~170 lines (`web/host-cef/src/notify_spike.rs` + 10 lines in renderer.rs/page.rs/main.rs):
  the JS shim, a V8 handler, one process message, a permission handler. Step 2 adds the protocol
  event `notify <id> ...` from the browser-process side (the tab is already known there).
- Run time: one small eval per script context: 629 us for the first context of a renderer, 162 us
  after (debug build, not under the perf lock; release will be less). Nothing per frame or per
  event.
- Detectable: `Notification.toString()` is not `[native code]`,
  `Object.getOwnPropertyDescriptor(window,'Notification').configurable === false`, `navigator` has
  an own `serviceWorker` property. Pages that replace `window.Notification` themselves (rare,
  mostly polyfills) cannot.
- Not covered:
  - **Notifications shown from inside a service worker** (`self.registration.showNotification` in
    the worker, e.g. after a `postMessage` from the page, and all push) and from dedicated workers:
    OnContextCreated is not called for worker contexts. Natively they stay at permission "default"
    and fail, so they are lost silently (nothing pops either). Push to closed pages is out of scope
    anyway (TODO). Which real sites notify only from their worker is unknown: checking needs logged-in
    sites (Slack, Gmail, Chat, WhatsApp Web), so it is left to the user's use of step 2.
  - `PushManager.subscribe` / `pushManager.permissionState()` still see the real permission
    ("prompt"); a site that subscribes to push after a "granted" may show its own error banner.
  - `onclick` on a notification: there is nowhere to click; step 2 could fire `click` when the user
    jumps to the buffer from `:notifications` (C-NOTIFLIST), later.

## Privacy

With the shim every site sees notification permission **"granted"** without asking. Effects:
- Sites that would show "enable notifications?" banners do not, and send notifications right away;
  they go only to the shell's bar and `:ls` (no sound, no desktop popup), so the cost is noise, not
  exposure. Nothing leaves the machine and no push subscription is made.
- Fingerprinting: "granted" everywhere differs from a typical Chrome profile ("default"), and the
  shim is detectable (above): web is more identifiable as "this browser" by sites that look. This
  is minor next to CEF's own UA/brands, but real.
- A site cannot tell whether the user actually saw anything (same as any granted browser).

## Recommendation for step 2

Build the **renderer shim** route, plus a native safety net:

1. Shim as in the spike (`notify_spike.rs` becomes `notify.rs`), always on, unless config says off.
2. Browser process: `web-host.notify` -> protocol event `notify <id> <title> <body>` (tag kept for
   replacing an unread notification of the same tag, frame URL/origin for C-NOTIFLIST's hostname;
   cap the title/body length in the host). Rate-limit per tab in the host or the shell.
3. Safety net: a `PermissionHandler` that answers every **notification** prompt with DENY at once
   (no Chrome bubble, and nothing can be granted natively, so Chrome's D-Bus bridge can never pop a
   desktop notification, even from a worker the shim does not reach). Other permission types keep
   today's handling (C-DL's item).
4. **Per-site option (cheap):** a config.rs setting `NOTIFY_SITES`: `All` (default: shim says
   "granted" everywhere) or a list of hostnames (listed: "granted"; others: "denied", constructor
   fires `error`, nothing forwarded). The list reaches the renderers as a switch added in
   `BrowserProcessHandler::OnBeforeChildProcessLaunch` (renderers start with it; changing it needs a
   restart, fine for a config constant). A runtime per-site prompt (`requestPermission()` asking the
   shell) is possible too (async process-message round trip back to the renderer to resolve the
   promise) but costs more; not recommended for step 2.

Prototype code: on this branch, **in the PR, off by default** (`WEB_HOST_NOTIFY_SPIKE` unset = no
permission handler, no shim; checked: run `off` identical to today). Remove or rename in step 2.

## Seen on the way (not C-BELL)

- Chrome's permission bubble is drawn for any prompt today (notifications here); the shell cannot
  drive it from the keyboard. Already covered by C-DL ("permission prompts as protocol events").
- First runs used `dbus-run-session` with the default session config: the private bus activated
  `xdg-desktop-portal-kde`, `kwalletd6` and `ksecretd` (on the nested display), and the first page
  load hung until they were gone; a KDE wallet password dialog appeared in the nest (nothing typed;
  the processes were killed by PID after checking their environment named the private bus). Something
  in the host's start asks KDE's wallet/secret service (Chromium's cookie-key store on KDE is
  KWallet, or the portal's Secret interface; not narrowed down). On the user's real bus that means
  the host may store its key in the user's wallet. Proposed item: check with
  `--password-store=basic` (and what it changes for cookie encryption in the throwaway profiles).
  `private-bus.conf` (no activation) avoids all of it in this tool.
