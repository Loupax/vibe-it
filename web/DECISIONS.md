# web — user decisions

A newer decision overrides an older one; add new ones at the top with the date.

## 2026-11-24 A mapping's `:` line runs with `<CR>`, otherwise it opens the line (vim's contract)

User: "I think we could have both 'prefill a command' and 'run a command' use the exact same interface";
"Yes, update them and drop :commandLine". `web.map`'s string rhs is one `:` line: ending in `<CR>` it runs,
without it the `:` line opens with that text typed. This changes SCRIPTING.md's earlier "a `:` string runs"
before C-SCRIPT-MAP is built, so nothing breaks. Only a trailing `<CR>` is a key: mappings never type keys,
so no `noremap`. `web.cmd` always runs. Items C-SCRIPT-MAP, C-EDITURL.

## 2026-11-24 Editing the page's URL in the `:` line

User: "I want an easy way to edit the currently visited page. I'm thinking `:go ` to automatically fill with
the currently open page, if it exists, and have it pre selected so the user can either wipe it clean, or move
the cursor to the end so they can alter the ending". Built into web, not a script: the script API has no hook
on `:` line edits and the line has no selection. Asked which commands and triggers: "Both go and new without
argument"; "do it for all commands that open URLs" (`go`, `new`, `bufferNew`, `openExternal`). Vim's Ctrl-R
`%` in the `:` line: "Let's skip this for now". A command that opens the `:` line with given text, so a
profile can map a key to it (qutebrowser's `:set-cmd-text`): "Yes, that sounds good"; later the same day
replaced by the map syntax (entry above), and dropped ("drop :commandLine"). Item C-EDITURL.

## 2026-11-24 Command-line history: Up/Down with a prefix filter, kept on disk

User: "When I'm entering a command I should be able to use up or down keys to navigate history. If already
typed something, that should be used as filtering."; then "Keep the command history on disk, like vim". The `:`
lines run are kept in the state directory (`DIR/cmdhistory`, STASH.md once built) and survive a restart, as
vim's shada keeps its cmdline history. Item C-CMDHIST (TODO.txt).

## 2026-11-24 Repeating the last command: vim's @:

User: "We need a command that repeats the last performed command, similar to vims `.`"; then "Create a @: and
we can map it to . on the scripting layer if we want. Let's stay as closer to the vim contract as possible so
muscle memory takes over". So web gets vim's `@:` (and `@@`, `:@:`), repeating `:` lines only, not vim's `.`;
`.` stays unbound and a profile can map it (`web.map("n", ".", ":@:")`). It is built in because SCRIPTING.md's
API cannot see which line ran last. Items C-CMDHIST, C-REPEAT (TODO.txt).

## 2026-11-24 Prompts are no longer logged

User: "I'm wondering if we should drop storing prompts altogether"; "update the relevant files so no prompt
gets updated anymore. Will remove it from main in a few days". No script or agent reads PROMPTS.md as input,
and it caused most merge conflicts (merge_pr.sh's keep-both rule). The why of a decision goes here, with the
user's words; an item's result goes in its PR body and commit message, large reports in `notes/`.
PROMPTS.md files are frozen until removed. Repo-wide: root README.md, COORDINATOR_PROMPT.md,
coordinator/AGENT_RULES.md, scratchy/AGENTS.md.

## 2026-11-24 web.cmd queues its line (C-SCRIPT)

The implementer's choice (C-SCRIPT), not asked. `web.cmd(line)` only appends the line to a queue; the shell
runs the lines a call queued right after that call into QuickJS returns. The alternative, running the command
inside `web.cmd`, needs the JS function to reach `Keys` (the shell's state) while `Keys` itself is in the middle
of calling into JS (`:Name` -> the user's function -> `web.cmd`): a `RefCell` borrowed across the call, or
`Keys` behind `Rc<RefCell<..>>` everywhere. With the queue no JS ever runs while the shell is borrowed and no
borrow is held across a call. Nothing is lost: a script cannot read anything back from a command (no getters,
SCRIPTING.md), and a queued line naming a user command runs that command's function before the next line, so
the order is the one synchronous calls would give. Lines queued at the top level of init.js wait for `ready`
(no engine host before it). Depth: user commands nested through `web.cmd` past `SCRIPT_DEPTH_MAX` stop the whole
chain with an error.

## 2026-11-24 User scripts: init.js per profile plus a global one, QuickJS in the shell

User: "I want to be able to expose configuration similar to how nvim does it. I'm thinking using
javascript"; "Setting hotkeys to :commands, Defining :commands, Attaching functions to hotkeys. That way I
can set my default buffers for example. I think we should have one init script file per profile, and maybe a
global." Spec: SCRIPTING.md.

- **Scripts set up a profile, not the app.** This narrows "no runtime config file" (KEYS.md "Configuration",
  2026-11-20) rather than reversing it: config.rs stays where web's own defaults and knobs live, edited and
  rebuilt; a profile's commands, keys and start buffers live in its script. User: "that's basically for
  setting user profiles, not for configuring the app itself."
- **QuickJS, in web-shell, nothing in the engine host.** User: "Let's keep everything split from the engine.
  The smallest the better. Go with QuickJS." V8 (scratchy's) was too large for a config file; running the
  script in the CEF host would put it in the pages' process and tie it to one engine.
- **The smallest API, and nothing exposed until the user asks for it:** `web.cmd`, `web.command`,
  `web.map`, `web.on("ready")`. Scripts act only through `:commands`. Getters (current URL, buffers, mode)
  and `unmap` were proposed and left out until needed. What `ready` passes is decided as it gets used.
- **`--profile NAME`**: the profile's script and its own state directory. Without it, the global script only.
- **No session restore.** User: "My goal is to replace that, and if we need it we might need to find a 'web'
  way to do it." A profile's `ready` handler opens its buffers at every start.

## 2026-11-24 Web Notifications go to the status bar, from every site by default

User: "We should plan a discreet notification feedback. Maybe a visual bell that causes the status bar to
flash? :ls should show which buffers have unread notifications"; "focus on catching notify events if
possible". Only a page's own Web Notifications count. The C-BELL spike (#727, notes/C-BELL-SPIKE.md) found
CEF exposes only the permission prompt and Chrome would send a granted notification straight to the desktop
over D-Bus; so the engine host replaces the notification API in every page and forwards each one to the
shell, and blocks Chromium's own notifications (no Chrome bubble, no desktop popup, even from a worker).
The shell flashes the bar, marks the buffer unread until it is shown, counts `[N!]` and marks `:ls`. No sound,
no desktop popups. **Default `NOTIFY_SITES = All`** (config.rs): picked by the coordinator per the spike's
recommendation, not by the user: every site sees permission "granted" without a prompt, so sites that
would ask first notify right away (the cost is noise in the bar, nothing leaves the machine), and "granted"
everywhere makes web a little more recognisable to sites that look. The user may switch to a list of
hostnames (`NotifySites::Only(&["chat.example.com"])`; every other site then sees "denied"). Notifications
sent from inside a worker (push, a service worker's own) are lost silently. Item C-BELL.

## 2026-11-24 :new opens a buffer

User: "we don't have an easy way to create a new buffer, add an action item for a :new command"; on the proposed
behaviour: "exactly what I wanted". `:new [url]` (alias `:bufferNew`) opens a buffer and makes it current, the
argument as `:go`'s; with no argument an about:blank buffer and the `:go ` prompt. The previous buffer becomes
the alternate (Ctrl-^). Item C-NEWBUF.

## 2026-11-23 H.264: build CEF ourselves; :openExternal as the escape hatch

User, after C-WEBEX (#718) showed Webex needs WebRTC H.264, which the prebuilt CEF lacks: "Let's do both.
Create :openExternal but let's also build chromium". C-EXTERNAL hands a page to another browser.
C-H264-BUILD builds CEF 156 with proprietary codecs (H.264/AAC, OpenH264) on this machine: the build may use
up to 20G of RAM (an exception to the 6G cap, for this build only; the machine has 31G), runs under nice/ionice
outside the repo (~/.cache/web-cef-build), and its output is never committed or published (codec patents).
C-H264 is unparked into C-H264-BUILD; DRM (C-DRM) stays parked.

## 2026-11-23 Modifiers reach the page on clicks in NORMAL

User: "we should pass the modifier keys on normal mod unless that breaks functionality". In NORMAL a
click carries the held modifiers (Ctrl/Shift-click behave as in INPUT: the host's no-new-tab rules apply),
while NORMAL's own bindings stay unaffected (the scroll keys it sends, e.g. G as End, must not arrive as
Shift+End). Item C-NORMALMODS.

## 2026-11-23 The picker's, bookmarks' and buffers' defaults are kept

User, on the choices agents made without asking ("they all look good right now"): the picker (C-PICK, KEYS.md
T58-T68: `>` prompt and `PICK history m/n` header, no wrap at the ends, Enter with no match does nothing, a
click on the list closes it and is kept from the page, words after the command prefill the filter, best
match on top) and bookmarks (C-BOOKMARKS: search newest first, delete by exact URL, a hand-edited file is
normalized on rewrite) stay as built; their *(proposed)* marks go.

## 2026-11-23 A picker, bookmarks, buffers as tabs, popups like Chrome

User, in a planning session (no code; PROMPTS.md "Picker, bookmarks, buffers, popups"). Overrides
"No tabs yet" and, for script popups and target=_blank, C-NOTAB.

- **Picker.** A list the shell draws over the page, above the status bar (the page is not resized),
  with fuzzy filtering, the same keys in every list (move, Enter runs the row, Esc closes) and a mode
  of its own. Each source gives rows of a label plus an action. It is for dedicated commands
  (`:historySearch`, `:bookmarkSearch`, `:bufferList`, `:popupSearch`; context-menu actions are a
  possible later source, C-CMDS stays parked). It never shows Tab candidates: "No menu" (KEYS.md,
  2026-11-20) was about `:go`'s completion and stays as it is.
- **Bookmarks.** A plain file `DIR/bookmarks` (`url<TAB>title`, editable by hand); `:bookmarkAdd`,
  `:bookmarkDelete [url]`, `:bookmarkSearch`; bookmarks also feed `:go`'s ghost, above history.
- **Buffers are the tabs.** One engine host holds several pages (the profile lock allows one host).
  Commands are camelCase with vim aliases (user: "go with b"): `:bufferList`/`:ls`,
  `:buffer`/`:b` (N or a partial name), `:bufferNext`/`:bn`, `:bufferPrev`/`:bp`, `:bufferKill`/`:bd`,
  `:bufferDescribe [text]` (no alias); Ctrl-^ for the previous buffer. Ghost completion offers the long
  names only. A buffer is searched and listed by title, hostname and an optional description the user
  sets ("keep it minimal": no page content, no snippets). The description stays across navigation and
  goes with the buffer. Killing the last buffer leaves an empty `about:blank` buffer ("The user doesn't
  know if the buffer they just killed is the last one"). Ctrl/middle-click on links still open nothing
  (middle-click amended 2026-11-24, see "Middle-click on a link opens a new buffer").
  Not now: searching page contents (maybe a separate feature), restoring buffers after a restart.
- **Popups follow Chrome** (user: "go with your suggestions"): a `window.open` from a user gesture opens
  a new buffer with `window.opener` kept (OAuth/payment logins work); one without a gesture is blocked,
  the status bar says `popup blocked: <url>` (or that it had no URL), and `:popupOpen` (latest) /
  `:popupSearch` (the buffer's list, cleared when it navigates away) open it in a new buffer. A
  `target=_blank` link opens a new buffer. Not now: a per-site "always allow popups" list.

## 2026-11-24 Middle-click on a link opens a new buffer (amends "Ctrl/middle-click on links still open nothing")

User (C-MIDCLICK): "Middle click on links should basically do a :new". A middle-click on a link now opens its URL in a
new buffer that becomes current, exactly as `:new <url>` does (T103: the old buffer is the alternate, Ctrl-^ returns),
in NORMAL and in INPUT (the new buffer starts in NORMAL). This reverses C-NOTAB for the middle button only: Ctrl-click,
Shift-click and Ctrl+Shift-click still open nothing, and middle-click on anything that is not a link is unchanged (no
paste, see below; no autoscroll). How the host tells a middle-click from Ctrl+click: Chromium gives both the same
disposition (NEW_BACKGROUND_TAB, no modifiers in `OnOpenURLFromTab`), so the host cannot there; the renderer's DOM can:
a middle-click is an `auxclick` with `button` 1 (Ctrl+click is a `click` with `button` 0 and `ctrlKey`). A script run
in every frame (`newtab::MIDDLE_JS`) cancels an unmodified middle-click on `a[href]`/`area[href]`, sends the link's
absolute URL to the host through a V8 function the script closes over (the page never sees it), and the host tells the
shell with `open-link`. The shell opens it like `:new` (no `window.opener`, no popup blocking: not a popup).

## 2026-11-23 No middle-click paste in the page; GTK stays

User: "didn't know I hade gtk dependencies in the first place. it's ok tho, Drop middle-click paste in the
page and keep GTK as it is." Chromium (libcef) loads GTK at run time for desktop integration: theme, dark
mode via prefers-color-scheme, and GTK settings such as gtk-enable-primary-paste, which GNOME ships off and
which gates middle-click paste into pages. web keeps GTK; there is no middle-click paste into the page. #689 closed.
(Corrected 2026-11-23 after #695: the coordinator first wrote that middle-click paste works in the `:` and
`/` lines; it was never built. Those lines paste the clipboard only: Ctrl-v, Ctrl-Shift-v, Shift-Insert.)

## 2026-11-23 Enter in NORMAL goes to the page; caret browsing as an experiment

User: "do this. Enter in NORMAL forwards to the page. Expose caret browsing from a command mode command,
so I can experiment with it but don't change the defaults." Enter in NORMAL is delivered to the page
(after a find, Chromium focuses the matched link, so Enter follows it). `:set caret` / `:set nocaret`
toggle Chromium's caret browsing; off by default. The rest of caret mode stays parked.

## 2026-11-23 Search places the pointer, Enter clicks

User: "On Search Mode n/N moves the cursor at the beginning of the match. Enter sends a click event to
it. I think that's better defined behavior." `/`+Enter, `n`, `N` move the mouse pointer to the start of
the current match; Enter in NORMAL clicks at the pointer, wherever it is (coordinator's reading: always,
not only after a search). Replaces KEYS.md S7/T18-T20 ("click the match", "Match spans N links"). Pairs
with C-CMDS, whose commands act on the element under the pointer. Item C5-ENTER.

## 2026-11-23 No context menu; its actions become commands

User: "maybe we could disable it completely, and expose it via the command mode" and, after the
coordinator's proposal (commands act on the element under the pointer now, picked by link hints later;
`:yank` needs the clipboard of C6), "yes, go ahead". C-NOMENU removes the menu (right-click still reaches
the page); C-CMDS adds `:save`, `:save link`, `:save image`, `:yank link`, `:yank image`. DRM (C-DRM) and
H.264 (C-H264) are parked (user, same day).

## 2026-11-24 Only a focus the user caused switches to INPUT

User (C-AUTOFOCUS): "moving to INPUT mode when input elements get focus confuses me [...] I'm thinking of not changing modes on autofocus, but should make sure that once I hit i, the autofocused input gets focus"; "include gi too". Amends "Focusing an editable switches to INPUT" (2026-11-23,
below): an `editable-focused true` switches to INPUT only when the user caused it, i.e. within
`CLICK_FOCUS_MS` of a mouse press, of a key the shell gave the page in NORMAL (Enter; the find +
Enter timed from the real press) or of `gi` (a click on "Reply" that makes the page focus a
textarea still counts). The page's own focus (`autofocus`, a late `el.focus()`) leaves the mode
alone, and the shell has the host blur the field and remember it ("parked"), so NORMAL's scroll
keys scroll the page; `i` focuses the parked field again. At most one blur per buffer per
`CLICK_FOCUS_MS` (pages that refocus on blur keep the focus). `gi` (as Vimium) focuses the last
text field, else the first visible one, and enters INPUT. Known effect: the blur runs the page's
blur handlers (a validation message, a closing suggestion list). KEYS.md T29, T32, T13, T6, T111.

## 2026-11-23 Focusing an editable switches to INPUT

**Amended 2026-11-24** ("Only a focus the user caused switches to INPUT", above): the page's own
focus no longer switches.

User: "when input elements gain focus, we should automatically switch to Input Mode. No auto-return on
blur since the exit of the form context isn't that clear." Any `editable-focused true` from the host
switches to INPUT, whether a click, the keyboard or page script (`autofocus`, `el.focus()`) caused it.
This reverses KEYS.md T32 (a page could not pull the user out of NORMAL). Blur never changes the mode.
Known effect: pages that autofocus a field (search engines, login pages) open in INPUT. Done in C3-RCLICK.

## 2026-11-23 Popups return to the previous mode

User, on C3's rule that a click opening a popup (context menu, select dropdown) switches to INPUT: "I
think it makes sense as UX, but what do you think about switching back to the previous mode after
clicking an option?" Agreed: INPUT while the popup is open, back to the previous mode when it closes,
unless the click focused an editable field. Item C3-RCLICK.

## 2026-11-23 forget-site uses DevTools Storage.clearDataForOrigin

User (on #670): "no need for hacks, use Storage.clearDataForOrigin. Make sure that's done in an
extendable manner for other engines." An in-process DevTools method call from the engine host
(CefBrowserHost::ExecuteDevToolsMethod) is allowed for forget-site; this is not the automation
protocol that "No browser extension, no automation protocol" excludes (no remote-debugging port,
no CDP socket, nothing outside the host). The forget contract sits behind an engine-neutral
interface so E1 (WebKit) implements it with its own API.

## 2026-11-23 No tabs yet

User: "Do not bother with tabs for now. Do not expose tab functionality yet. Maybe we'll mimic
them using the buffers interface, but let's not commit now." C4 is deferred (not scheduled, no
design session). The engine host shows one page: protocol v0 has no `new-tab`/`close-tab`, and
`window.open`/target=_blank load in the same page (coordinator's reading, sent to C1; known loss:
popups that rely on `window.opener`, e.g. OAuth login popups). Events keep their `<id>` field so
a later buffers or tabs design needs no protocol change.

## 2026-11-23 The proxy is optional

User: "do not have it require a proxy. It should support it but a missing --proxy flag
should default to standard connectivity". Without `--proxy`, the engine uses its normal
networking; a missing flag is never an error or a warning.

## 2026-11-23 No enforcement in web

User: "don't enforce that. It's something we can do separately". web passes a proxy to
the engine (`--proxy <host:port>`, handed to the engine host; without it the engine
connects directly) and does nothing else about the network. Keeping a program off the
network except through a proxy (an empty network namespace plus a forwarder to the
proxy's socket, as the spike's W2 did) is a separate, engine-neutral wrapper that can be
put around web, or any other program, from outside. Overrides "Enforcement: an empty
network namespace" below and the namespace parts of "Architecture C". Paths that ignore
a proxy setting (WebRTC's UDP, for one) are not web's concern.

## 2026-11-23 Blink first

User: "let's begin with Blink so we can have something working as soon as possible. We
can switch to a more lightweight engine later". web-host-cef is the only engine host
through C8; web-host-webkit (or another lighter engine) is E1, after C8. The control
protocol stays engine-neutral so E1 needs no shell change.

## 2026-11-23 The engine owns localStorage; scratchy's stash store is dropped

User: "drop it and record the decision. Have the engine deal with localStorage unless we
have a use case for centralizing it". localStorage, sessionStorage and IndexedDB live in
each engine host's profile (STASH.md "State directory"), in the engine's own format.
scratchy's one-file-per-key store, its locks and generation counter are not carried
over. Reopen only with a concrete reason to keep page storage outside the engine (e.g.
sharing it between engines or editing it by hand).

## 2026-11-23 Architecture C: compositor shell, engine hosts, a web proxy

From the discussion after the spike (PROMPTS.md "Architecture C"). Overrides "Request
path" and "Engine" below; "Cookies: Chromium owns the jar" and "Enforcement: an empty
network namespace" still hold.

- **Network: a web proxy.** web needs one thing from the network side: an HTTP forward
  proxy (plain requests and `CONNECT`) reachable on a Unix socket. Which proxy that is
  is the user's choice, not web's. The engine host gets a proxy setting pointing at a
  forwarder inside its empty network namespace, which reaches that socket. Nothing
  engine-specific sits on the network path, so the namespace plus a proxy setting is
  the whole contract. W3's resource handler and its wire client leave the browser.
- **Shell: web-shell is a Wayland compositor** (smithay, a nested compositor shown as
  one window on the user's desktop). It draws the status bar and command line, owns all
  keys and pointer input, and forwards to the page what the mode allows.
- **Engine: separate engine-host programs, own protocol.** Each engine gets a small host
  with no browser UI (one window per tab, the page only) and a control socket speaking
  our protocol. `web-host-cef` first (from the spike), `web-host-webkit` (WebKitGTK)
  second. The engine is picked at start (`--engine`); no switching while running, and
  logins do not carry between engines (each keeps its own cookies).
- **Rejected:** a browser extension as the shell (the user does not want to rely on the
  extension API); driving a stock browser over CDP or WebDriver BiDi (automation flags,
  sites that detect it); off-screen rendering into the shell (rebuilds popups, IME,
  drag and drop that the engine does natively as a Wayland client).
- **X11:** web-shell runs on an X11 desktop too: its winit backend opens an X11 window
  there, and the engine hosts keep speaking Wayland to the shell. C2's check covers both.
- **Carry over from scratchy everything that can be carried** (user): KEYS.md and the
  STASH.md now live in web/ (KEYS.md, STASH.md: state directory, history, forget-site,
  cookies); the shell code to port is
  listed in TODO.txt "From scratchy".

## 2026-11-23 The spike is a success; testing stops

The user, after W1-W5 (W4 prepared, W5 ready) and with W6 in progress: "I consider that a
success. you can stop testing now". W6 was stopped; its partial work (switch lists, start of a
WebSocket proxy) is on branch `web/w6-bypass`. W4's real-site login and W7's day of browsing
are not run. Whether this is the go for the "After a go" items is asked separately.

## 2026-11-23 Start `web`, a CEF-based browser; pause scratchy

From the engine discussion of 2026-11-23 (PROMPTS.md "Engine discussion").

- **Priority: feature completeness first, wire compatibility second.** The goal is a
  daily-driver browser; the browser shell is the full-time job, not the engine.
- **Engine: Chromium (Blink) through CEF, prebuilt binaries.** Servo (~62 % overall WPT
  score, Feb 2026) and Ladybird (pre-release) are not daily-driver complete. WebKitGTK
  has no hook to answer http(s) requests from our own code. Firefox has no desktop
  embedding API.
- **Bindings: the `cef` Rust crate** (Tauri team), so the shell stays in Rust.
- **Request path:** a `CefResourceRequestHandler` / `GetResourceHandler` answers every
  http(s) request by sending it to wire's Unix socket and streaming the response back.
- **Cookies: Chromium owns the jar** (proposed; to confirm in the spike). `document.cookie`
  reads Chromium's jar, so a wire-owned jar breaks sites. wire passes `Set-Cookie`
  through and the browser sends no `X-Wire-Jar`. wire's jar stays for other clients.
- **wire keeps:** TLS, DNS, redirects, decompression, request policy, cache, logging,
  and blocking (ads/trackers).
- **Chromium enforces** CORS, mixed content and Secure contexts; scratchy's versions
  (`js_net.rs`) are not ported.
- **Enforcement: an empty network namespace.** The whole CEF process tree runs with no
  network interfaces (`unshare -rn` or bubblewrap `--unshare-net`); wire runs outside it.
  A Unix socket addressed by file path still works across network namespaces, so wire
  is the only way out and any path the handler misses fails instead of leaking. This
  replaces scratchy's seccomp filter, which cannot cover CEF's child processes.
- **Known losses accepted up front:** WebRTC (own UDP sockets) cannot go through wire.
  WebSockets, QUIC, DNS prefetch, preconnect and service-worker fetches are checked in
  the spike (TODO.txt step 6).
- **First step is docs only;** no CEF download or code until the spike is started.
