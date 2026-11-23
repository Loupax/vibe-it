# C4-SPIKE: several pages in one host, popups as pages, blocked popups

Report for web/TODO.txt C4-SPIKE (2026-11-23). The protocol proposal it leads to is in
PROTOCOL.md "Proposal: several pages (C4, C-POPUPS)". Spike only: nothing here is built into
web-host-cef or the shell yet.

Experiment code (not for merge) is on branch `web/c4-spike-exp` (commit 62506c5a):
`web/host-cef/src/page/spike.rs` (hooked into page.rs, enabled by `WEB_HOST_C4=views|windows`,
commands `c4 ...` on the control socket), `web/spike/c4/c4drive.py` (plays the shell, serves the
pages, real clicks with web/tools/inject) and `web/spike/c4/pages/` (probe, opener, frame, child).
All runs: CEF 156 (cef crate 156.7.0), release build, headless nested dwl (web/tools/nest.sh,
960x1006), every host in a 6G memory-capped scope, throwaway profile under `.qwork/`, `--mute-audio`.
The web shell was not used or changed: dwl played the compositor (see "What was not tested").

## Answers in short

1. **Several browsers in one host: yes, one top-level window per page.** Several Chrome-style
   BrowserViews in ONE window is impossible in CEF 156 (`Cannot add multiple Chrome style
   BrowserViews`, also after removing the first one), so "the host swaps pages inside its one
   window" is out without switching to Alloy style. Each page as its own window works:
   `tab-created <id>` goes out before that window is shown, as for the first page today.
   **A page the shell merely does not draw keeps running at full speed**; the host must hide it
   itself (`BrowserView::SetVisible(false)` in its own window): then it is a hidden page as in
   Chrome's background tabs (rAF stops, timers 1/s, audio keeps playing). Memory is not freed.
2. **Gesture popups become pages with a working `window.opener`: yes.** `OnBeforePopup` returns 0,
   `OnPopupBrowserViewCreated` puts the popup's BrowserView in a new window of ours (returns 1), the
   popup is a new browser (`tab-created`), it `postMessage`s to its opener and `window.close()` in it
   closes only it. CEF does say whether there was a gesture: `user_gesture` 1 for real clicks, 0
   for timers. **Blocked popups:** with Chromium's own blocker the host gets no CEF callback, only
   DevTools `Page.windowOpen` (`userGesture:false`). Simpler and complete: run with
   `--disable-popup-blocking` and let the host be the blocker: every `window.open` then reaches
   `OnBeforePopup` with `user_gesture`, the frame, URL, name and features; the host cancels the
   gesture-less ones (the page gets `null`, as with Chrome's blocker) and reports them.
3. **`:popupOpen` can re-run the original `window.open` in the frame that asked, opener kept:
   yes**, also in an iframe. The host keeps the frame id; `:popupOpen` runs
   `window.open(url, name, features)` in that frame (`CefFrame::ExecuteJavaScript`) with a one-shot
   "allow this frame + URL" so the host's own blocker lets it through. The child got
   `window.opener` and its message reached the asking frame. With Chromium's blocker instead,
   DevTools `Runtime.evaluate` with `userGesture:true` also gets past it (main frame checked).
   If the frame is gone, load the URL fresh in a new page (no opener).

## 1. Several pages in one host

### One window, several views: not possible in Chrome style

`c4 new <url>` created a second `BrowserView` and added it to the first page's window:
`ERROR:cef/libcef/browser/views/browser_view_impl.cc:324] Cannot add multiple Chrome style
BrowserViews` and the view stayed empty (screenshot: a blank window; no browser was ever created,
no `on_after_created`). Removing the first view (`Window::RemoveAllChildViews`) before adding the
new one gives the same error: a Chrome-style CEF window is one Chrome `Browser` with one tab.
Re-adding the original view works (two `NOTREACHED ... Observers can only be added once!` lines,
no visible effect). Alloy-style browsers (`BrowserViewDelegate::GetBrowserRuntimeStyle`) would
allow several views per window, but the host relies on Chrome style (no Chrome UI, Chrome's
command handling, C-BGNET's switches); not tried.

### One window per page: works

`WEB_HOST_C4=windows`: every page (`c4 new`, popups) gets `window_create_top_level` with the same
`HostWindowDelegate` as today (app_id `web-host-cef`); `announce_and_show` sends `tab-created <id>`
and then shows the window. Log (e1-win):

    5.06 EV {"event":"tab-created","tab":2}
    5.06 c4: window of tab 2 shown

Ids are CEF browser identifiers (1, 2, 3, ...; the forget-site helper browser takes one too, so
they need not be contiguous). Pairing for the shell: each page window is shown after its
`tab-created`, windows are shown in the order of their `tab-created`s, and both streams are
ordered (one Wayland connection, one control socket), so a FIFO of announced-but-unpaired tabs
pairs every new toplevel without a parent. A toplevel that arrives while no `tab-created` is
waiting is not a page (a dialog), as today.

### What a page that is not shown does

The probe page logs once a second: `visibilityState`, `hasFocus()`, requestAnimationFrame calls/s,
a 100 ms `setInterval`'s ticks/s (over the measured elapsed time) and an AudioContext oscillator's
state and `currentTime`. Steady values after ~1 s:

| how the page is "not shown" | visibility | rAF/s | 100 ms timer/s | audio |
|---|---|---|---|---|
| shown (reference) | visible | 60 | 10 | running |
| window mapped, compositor does not draw it (dwl: moved to another tag; dwl also sends `xdg_toplevel` `suspended`, and wlroots sends it no frame callbacks) | **visible** | **60** | **10** | running |
| host: `BrowserView::SetVisible(false)` in its own window (window stays mapped) | hidden | 0 | 1 | running, `currentTime` keeps advancing |
| host: BrowserView removed from its window | hidden | 0 | 1 | running |
| host: `Window::Hide()` (unmaps) | hidden | 0 | 1 | running |
| host: `Window::Minimize()` | hidden | 0 | 1 | running |

(e1-win, e1-tag; `visibilitychange` fires at once in each host case and back on show; `focus`/
`blur` follow keyboard focus.) So the compositor alone cannot make a page "background": Chromium
156 under Ozone/Wayland ignores `suspended` and keeps its own frame clock without frame
callbacks. The host must hide every page but the shown one; `SetVisible` on the BrowserView is the
one that keeps the window mapped and paired (Hide unmaps it: the shell would see the toplevel go
and come back). Audio keeps playing in hidden pages (Chrome's behaviour; a video page keeps
sounding when you switch away). Chrome's "intensive" throttling (one wake-up per minute after 5
minutes hidden, not for audible pages) was not waited for.

Memory (e5-mem, probe pages, scope `memory.current` / processes):

| pages | memory.current | anon | processes |
|---|---|---|---|
| 1 | 159 MiB | 130 MiB | 10 |
| 2 | 171 MiB | 141 MiB | 11 |
| 3 | 190 MiB | 151 MiB | 12 |
| 4 | 203 MiB | 163 MiB | 13 |
| 4, three hidden 5 s | 203 MiB | 163 MiB | 13 |
| 2 (two closed) | 178 MiB | 147 MiB | 11 |

One renderer process per page (same site too), about 11-19 MiB per trivial page; hiding frees
nothing (CEF has no tab discarding), closing does. (uptime: load 1.72.)

Also seen: the host's `quit` closed only the first page (today's `on_before_close` quits the
message loop when *the* page closes) and the second window stayed until the scope was stopped. C4
must close every browser on `quit` and exit when the last one is gone.

## 2. Popups as pages

Real clicks (inject, headless dwl) on opener.html. With `WEB_HOST_C4=windows`, Chromium's blocker
on (e2-chrome) and with the host as blocker (e3-host) alike:

| action | OnBeforePopup | result |
|---|---|---|
| click, `window.open(u,'kid','popup,width=400,height=300')` | `NEW_POPUP`, gesture 1, features popup=1 w=400 h=300, name `kid` | new browser 2, own window, `tab-created 2`; opener gets a window; child: `opener=yes name="kid"`; opener got `hello from child gesture` |
| click, `target=_blank` link | `NEW_FOREGROUND_TAB`, gesture 1, name `` | new page; `opener=null` (Chromium's implicit `noopener` for `target=_blank`, as in Chrome) |
| click, `window.open(u,'_blank','noopener')` | `NEW_FOREGROUND_TAB`, gesture 1 | new page, `opener=null`; `window.open` returned null (spec) |
| click on the child's `window.close()` button | | the child's browser closes (`on_before_close`, `tab-closed 2`), the opener stays |
| timer at load (main frame), timer in an iframe, timer 7 s after a click | Chromium blocker: **none**. Host blocker: gesture 0 (frame id, URL, name, features given) | `window.open` returned null in both modes |

The popup's browser inherits the opener's `HostClient` (the same handlers), and
`OnPopupBrowserViewCreated(popup_view)` returning 1 after
`window_create_top_level(HostWindowDelegate(popup_view))` gives it our window (app_id
`web-host-cef`, no Chrome UI). `no_javascript_access` came in as 0 for every popup, also for
`noopener` (Chromium handles noopener itself; nothing for the host to do).

### Learning about a blocked popup

- Chromium's own blocker (today's setup: no `--disable-popup-blocking`) blocks in the browser
  process before CEF's `OnBeforePopup` and tells CEF nothing: no CEF callback, no console message
  (C1-INPUT saw the same). The only trace found is DevTools: with `Page.enable` on the page
  (in-process, `CefBrowserHost::SendDevToolsMessage` + a `CefDevToolsMessageObserver`, the
  mechanism forget-site uses), every `window.open` fires `Page.windowOpen`
  `{"url","windowName","windowFeatures":[...],"userGesture":false}` (also from the iframe), before
  `OnBeforePopup` for allowed ones. It gives no frame id, so a re-run cannot target an iframe, and
  every page needs a DevTools session.
- **Host as the blocker** (`--disable-popup-blocking`, then `user_gesture == 0` -> cancel and
  report): every `window.open` and `target=_blank` reaches `OnBeforePopup` with the frame
  (`CefFrame::GetIdentifier`, URL, main or not), URL, target name, disposition and
  `CefPopupFeatures` (popup flag, x/y/width/height). Returning 1 makes `window.open` return null,
  exactly what the page sees from Chromium's blocker. In every case run here `user_gesture` matched
  what Chromium's blocker did (allowed: 1, blocked: 0); it should be the same bit (the page's
  transient activation, not checked in Chromium's source). This is
  what the proposal uses. C1's drive.py already runs with `--disable-popup-blocking`, so README and
  drive.py's claim that it is "needed" becomes literally true, for a different reason.

## 3. Re-running a blocked popup (`:popupOpen`)

Host blocker (e3-host): the frame timer in the iframe and, later, the 7 s main-frame timer were
blocked and recorded; `c4 popup-open js` then ran

    window.open("…/child.html?n=frametimer", "fkid", "")

in the recorded frame (`CefBrowser::GetFrameByIdentifier` + `ExecuteJavaScript`) after setting a
one-shot allow (frame id + URL). `OnBeforePopup` came 1 ms later with gesture 0, the allow matched
and was used up, the popup became browser 2 in its own window, `window.open` returned a window,
the child logged `opener=yes name="fkid"`, and the **iframe** logged `message from … hello from
child frametimer`. Same for the main frame (`hello from child late`; the opener's title changed).

Chromium blocker (e4-dt): DevTools `Runtime.evaluate` with `userGesture: true` and the same
`window.open` in the main frame reached `OnBeforePopup` with gesture 1 and opened browser 2 with a
working opener. A subframe would need the frame's execution context id (from
`Runtime.executionContextCreated`); not tried, the host-blocker way needs none of it.

Limits of a re-run (both ways):
- The page's original call already returned null; the page's own variable never gets the
  handle. Flows that only need `window.opener` in the popup (OAuth/payment: the popup posts back
  or calls `opener.…`) work; a page that kept polling its handle (`w.closed`) does not.
- It runs the page's current `window.open` in the page's main world: a page that replaced
  `window.open` gets its own function called. The features string is rebuilt from
  `CefPopupFeatures` (`popup` plus size), not the original text; sizes are ignored anyway (C-POPUPS).
- The frame must still be there: the blocked list is cleared when the buffer navigates
  (C-POPUPS), and a frame that went away (iframe removed or navigated) makes the host load the
  URL fresh in a new page (no opener). A popup with no URL (`window.open()` + `document.write`)
  re-runs as `window.open('', name)`: the page's later writes are gone, it shows a blank page.
- The one-shot allow must not linger: clear it when the popup comes or after ~1 s.

## Other findings for C4 / C-POPUPS

- Chrome-style popup windows have no `xdg_toplevel` parent, so today's `pairing.rs` would call a
  popup's window a "dialog"; with the proposal every window announced by `tab-created` is a page.
- C1-INPUT's open cases (ctrl/shift-click reach the page without modifiers through the shell,
  middle-click dropped by the host) were not re-run here; they are input-path questions for
  C-POPUPS and do not change the protocol.
- The shell sizes every page window to the page area (also hidden ones), so switching does not
  re-layout the page.

## What was not tested

- The web shell itself: dwl played the compositor. The "not drawn" row above is dwl with
  `suspended` + no frame callbacks; a smithay shell that sends neither should behave the same
  (not checked).
- Alloy-style views (several per window), Chrome's own tab strip/`ExecuteChromeCommand` tab
  switching, the 5-minute intensive throttling, renderer-process limits with many tabs, and X11.
