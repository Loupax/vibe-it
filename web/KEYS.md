# Keys: input mode, normal mode, search

Carried over from `../scratchy/KEYS.md` (2026-11-23, decision "Architecture C" in
DECISIONS.md). The modes, rows, numbers and decisions are scratchy's, unchanged unless a
note marked **web:** says otherwise. Item names in the decision log (QKEYS-D, QSEARCH-D,
QCOMP-D, ...) refer to `../scratchy/TODO.txt`. "scratchy" below means the shell, `web-shell`.

**web: who sees the keys.** `web-shell` is the Wayland compositor the engine host's
windows live in, so it receives every key and pointer event first and decides what the
host window gets (`wl_keyboard` / `wl_pointer`). "Delivered to the page" means "the
compositor forwards the event to the host window"; "not delivered" means the host never
sees it. No engine API is needed for that. What the compositor cannot know by itself
comes from the host over the control socket (web/TODO.txt C1): which element has focus
after a click (T29, T32), and find-in-page (Search).

Status: **built in web by C3** (`shell/src/keys.rs`, `config.rs`, `completion.rs`,
`registers.rs`, `statusbar.rs`; tests named after the rows in `shell/src/tests/`, live
check `shell/test/c3_live.py`), Search built by C5 (`shell/src/tests/search_tests.rs`, live
check `shell/test/c5_live.py`), clipboard (`@+`, `yy`, paste) built by C6 (`shell/src/tests/c6_tests.rs`, live check `shell/test/c6_live.py`), Enter in NORMAL by C-ENTER (`shell/src/tests/c_enter_tests.rs`, live
check `shell/test/c_enter_live.py`), the picker by C-PICK (`shell/src/picker.rs`, `shell/src/tests/c_pick_tests.rs`,
live check `shell/test/c_pick_live.py`), buffers by C4 (`shell/src/buffers.rs`, `shell/src/tests/c4_tests.rs`, live check
`shell/test/c4_live.py`). **This file is the source of truth** for web's keys: when web/TODO.txt and this
file disagree, this file wins. Rows marked *(proposed)* are Claude's
proposals that the user has not confirmed yet; they are built as written unless the
user changes them before the item starts.

## Why

Before this design, every key went to the page's `keydown` handlers first, and any key
the page did not cancel fell through to scratchy's bare `j k h l g G :`. That fails both
ways: scratchy steals keys pages use without `preventDefault` (YouTube `l` seeks *and*
goes forward in history), and pages steal scratchy's keys (vim.js on social.loupax.com
cancels `:`). The fix is to give the keyboard to exactly one side at a time.

Rejected triggers for switching: holding Ctrl (pages and text fields use Ctrl chords:
Ctrl-K, Ctrl-Enter, Ctrl-A/C/V); tapping a lone modifier twice (impossible with
home-row mods, e.g. a Corne where Ctrl is a held `a`, or a tap-hold Caps Lock).

## Modes

| Mode | Status bar | Kind | Who gets the keys |
|---|---|---|---|
| INPUT | `INPUT` | sticky | the page ("input passthrough"), then the UA defaults |
| NORMAL | `NORMAL` | sticky | scratchy's bindings; the page gets no key events *(proposed, P2)* |
| SEARCH | `/` + the query | one line | the status-bar line editor |
| COMMAND | `:` + the command | one line | the status-bar line editor |
| PICK | `>` + the filter; the list over the page shows `PICK` | one line + a list | the status-bar line editor and the picker's keys (C-PICK, "Picker") |

Two short-lived helper states hold the timing rules: **ESC1** (in INPUT, one Esc seen,
waiting for a second) and **G** (in NORMAL, one `g` seen, waiting for a second). The
status bar shows their parent mode. *(Planned, QREG1: a third, **Y**, one `y` seen.)*

- scratchy **starts in NORMAL** (user, 2026-11-21, QKEYS5; D4 said "starts in input mode (for now)"). The
  `START_MODE` knob in `src/config.rs` sets it; a click into a text control still switches to INPUT.
- **Sticky** means a mode lasts until a row below changes it. In particular a
  navigation never changes the mode (`h h h` in NORMAL walks back through history).
- The mouse always goes to the page, in every mode.

```mermaid
stateDiagram-v2
    [*] --> INPUT
    INPUT --> ESC1: Esc (page gets it)
    ESC1 --> NORMAL: Esc within 300 ms (page does not get it)
    ESC1 --> INPUT: other key or too late (page gets the key)
    NORMAL --> INPUT: i, gi, or a click into a text field or select
    NORMAL --> SEARCH: /
    NORMAL --> COMMAND: :
    NORMAL --> G: g
    G --> NORMAL: g = top, other key = handled as in NORMAL
    SEARCH --> NORMAL: Enter = go to the first match, Esc = cancel
    COMMAND --> NORMAL: Enter = run, Esc = cancel
    SEARCH --> INPUT: click into a text field or select
    COMMAND --> INPUT: click into a text field or select
    COMMAND --> PICK: Enter on a picker command (historySearch)
    PICK --> NORMAL: Enter = run the row, Esc or a click = close
    PICK --> INPUT: an editable gains focus
```

## Transition table

"Esc" means Escape with no Ctrl, Alt or Super held. "Key repeat" is an OS auto-repeat
event (`repeat: true`). `ESC_ESC_MS` = 300, a const next to `resolve_binding` (dwm
`config.h` style: a QMK user can swap the trigger for F13 in one line). Each row is
meant to become at least one test; tests name the row (`keys_t07_...`).

### INPUT and ESC1

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T1 | INPUT | Esc press | not a repeat | ESC1 | deliver to the page as usual (it may close a modal, blur a text field); remember the time |
| T2 | ESC1 | Esc press | within `ESC_ESC_MS` of T1, not a repeat | NORMAL | NOT delivered to the page. Counts even if the page cancelled the first Esc *(proposed, P3)* |
| T3 | ESC1 | Esc press | later than `ESC_ESC_MS` | ESC1 | treat as a new first Esc (T1), new time |
| T4 | ESC1 | any other key press, or an Esc repeat | none | INPUT | handle the key as in INPUT (T5) |
| T5 | INPUT | any other key press/release | none | INPUT | forwarded to the host window; the engine does the rest (page handlers, text fields, selects, media, Tab focus, its own scroll keys). scratchy's bindings are never consulted. **web:** scratchy's own order list is gone, the engine owns it |

The ESC1 timeout is checked lazily, from the time stamp, when the next key arrives; no
timer is needed. The pure function is `resolve_esc_esc(state, key, repeat, now)`.
**web:** smithay's winit backend drops the OS key repeats, so the shell sees one press per
key; a press of a key already down is treated as a repeat (`Keys::key`).

### NORMAL and G

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T6 | NORMAL | `i` | none | INPUT | nothing else. **web, 2026-11-24 (user, C-AUTOFOCUS):** if the current buffer has a parked field (T32), the host focuses it again (`focus-field last`), so typing lands in the field the page picked. A park ends at a cross-document load and at any later editable focus, so `i` after Esc Esc out of a field you clicked does not jump to an old one |
| T7 | NORMAL | Esc | none | NORMAL | clear search highlights (the query is kept). Never changes mode. **web:** sends `find-clear` only when a find session is open; the match stays selected in the page (host), the bar's `[i/N]` goes; `n` afterwards starts a new session where the engine picks (usually the match after the one that was active) |
| T8 | NORMAL | `/` | none | SEARCH | empty search line. **web:** built (C5): the bar shows `/` and the query like the `:` line (no ghost, Tab does nothing) |
| T9 | NORMAL | `:` | none | COMMAND | empty command line |
| T10 | NORMAL | `g` | none | G | nothing |
| T11 | G | `g` | none | NORMAL | scroll to the top |
| T12 | G | any other key | none | NORMAL | handle that key as in NORMAL (it may itself be `:`, `/`, `j`, ...; `i` is `gi`, T111) |
| T111 | G | `i` (`gi`, as Vimium) | none | NORMAL, then INPUT when the focus arrives | **2026-11-24 (user, C-AUTOFOCUS: "include gi too"):** focus the last text field focused in this document (the parked one included) if it is still connected and rendered, else the first visible text field (input text/search/url/tel/email/password/number, textarea, contenteditable; not disabled or read-only; the first in the viewport, else the first in the document). INPUT when a field has the focus (also one that already had it). No field: status bar `gi: no text field on this page`, the mode stays. **web:** `focus-field last`, its `field-result` false -> `focus-field first`; a `KEY_SEQUENCES` row. The first field is looked for in the main frame only (not in iframes or shadow roots) |
| T54 | NORMAL | `y` | none | Y | nothing |
| T55 | Y | `y` | not a repeat | NORMAL | copy the URL to the system clipboard, same as `:let @+ = @%` (see "Registers"). **web:** built (C6) |
| T56 | G or Y | a key that is not the sequence's second key | none | NORMAL | as T12: handled as in NORMAL, so `g` `y` leaves Y pending and `y` `g` leaves G pending |
| T57 | INPUT, entered by T29 from a popup | the popup closes (option picked, Esc, click outside; the xdg_popup is destroyed) | the click did not focus a field | the mode before the click | the user's decision (DECISIONS.md, "Popups return to the previous mode"). A click that focused a field (`editable-focused true`) stays INPUT. Nested popups (submenus) are one session: the return happens when the last one closes. A popup with no user click (T32) never changed the mode, so nothing returns. An open `:` or `/` line was cancelled by T29, so the return is plain NORMAL. **web:** `Keys::popup_opened` / `popup_closed`, called from the shell's `new_popup` / `popup_destroyed`; `popup_return` holds the mode, `user_focused_control` (the `editable-focused` path) clears it |
| T13 | NORMAL | `j` `k` Ctrl-d Ctrl-u `G` Home End | none | NORMAL | scroll (repeats keep scrolling). **web:** done by sending the host window real input (arrow/PageDown/Home/End keys or wheel steps), not through a page API; which event each binding becomes is a `config.rs` table. Built (C3): `SCROLL_INPUTS`; a key row (`ScrollInput::Key`) is pressed in the page window while the bound key is held and released with it, so the engine's own key repeat keeps scrolling; a `ScrollInput::Wheel(n)` row sends wheel steps at the pointer (only over the page). Defaults: `j`/`k`/arrows -> arrow keys, Ctrl-d/Space/PageDown -> PageDown, Ctrl-u/Shift-Space/PageUp -> PageUp (Chromium has no half-page key), `gg`/Home -> Home, `G`/End -> End. A key goes to whatever has the focus. **Changed 2026-11-24 (user, C-AUTOFOCUS):** a field the page focuses itself is blurred (parked, T32), so `j` scrolls the page instead of moving that field's caret. A field the user focused and left with Esc Esc keeps the focus (as before), and so does a field of a page that refocuses on blur (T32's guard): there `j` moves the caret |
| T14 | NORMAL | `h` `l` | not a repeat | NORMAL | history back / forward (**web:** `back` / `forward` on the control socket) |
| T15 | NORMAL | `n` / `N` | a query exists, it has matches on this page | NORMAL | next / previous match in document order, wrapping with "search hit BOTTOM, continuing at TOP" (or TOP/BOTTOM); bring the screen to it (S4). **web:** `find <query> next|prev` (`n` = next, `N` = prev), the same query so the host steps; the wrap is derived from the `find-result` indices (next: new index not above the old one; previous: not below), and the bar shows `[i/N]`, after the wrap message when there is one (`search hit BOTTOM, continuing at TOP  [1/5]`). One match: every press wraps. Replies are matched to presses in order. No session on this page yet (after Esc or a navigation): the engine starts a new one, no wrap is told |
| T16 | NORMAL | `n` / `N` | a query exists, no matches here | NORMAL | status bar: `Pattern not found: <query>`. **web:** `find-result` with count 0 |
| T17 | NORMAL | `n` / `N` | no query yet | NORMAL | status bar: `No previous search` |
| T18 | NORMAL | Enter (no modifier; keypad Enter too) | a find session is open (after `/`, `n`, `N`; also when the last search found nothing) | NORMAL | end the find as T7 does (`find-clear`: the highlights and `[i/N]` go, the query is kept), then give the page an Enter `ENTER_AFTER_FIND_CLEAR_MS` (50) later, held until the real key is released. No shell-side click: the engine decides. Ending the find is what makes Chromium focus the link or control holding the active match (or select the match's text), so the Enter follows that link or presses that button. A match in a text field: the field gets focus, T29 switches to INPUT (C-ENTER live check) |
| T19 | NORMAL | Enter | no find session | NORMAL | the press and its release are delivered to the page, as in INPUT; the engine acts on whatever has focus (a link or button focused by an earlier find + Esc, by Tab in INPUT, by the page, or by caret browsing's caret). Nothing focused: the page gets a `keydown` and nothing happens |
| T20 | NORMAL | Ctrl/Alt/Super/Shift + Enter | none | NORMAL | nothing (T21) |
| T21 | NORMAL | any other key | none | NORMAL | nothing. Not delivered to the page *(proposed, P2)*; that includes Tab and Space, and keys for a focused text field or video. Enter is the exception (T18/T19, user 2026-11-23) |

### SEARCH and COMMAND

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T22 | SEARCH | text editing keys, paste | repeats (C-BARREPEAT) | SEARCH | edit the query (`TextEditBuffer`, same as `:`); paste as in "Paste" below |
| T23 | SEARCH | Enter | query not empty | NORMAL | the query becomes the kept query; find matches; current = the first match whose top is at or below the top of the screen, in document order (none below: wrap to the first match, with the wrap message); bring the screen to it (S4); `[i/N]` in the status bar. No matches: T16's message. **web (C5b, exact):** the shell sends `find-clear` (if a session is open) and then the new query with `screen_top`, so the same query again starts fresh. Chromium's find starts from the selection, and left alone CEF picked its own first match (with the screen at y=1710 and matches at 1300/2200/..., match 2, the one above the screen). So the host first collapses the selection at the first text (text node or text-field value, in document order) with a line whose top is at or below the top of the screen (`host-cef/src/find_start.js`) and then starts the find: the current match is the first one at or below the top of the screen. None at or below, one above: the selection goes to the document start, the first match is current and the host's `find-result` says `wrapped`, so the bar shows `search hit BOTTOM, continuing at TOP  [1/N]`. Only `/` Enter does this; `n`/`N` step the open session, and `n` after Esc or a navigation still starts where the engine picks (T7, T15). Limits: matches split over elements, in shadow DOM or in iframes are not seen by the page walk (the engine still finds them; no wrap is claimed for them) |
| T24 | SEARCH | Enter | query empty | NORMAL | same as Esc (T25) |
| T25 | SEARCH | Esc | none | NORMAL | cancel: highlights and the kept query as they were before `/`. **web:** nothing is sent while typing, so nothing has to be restored |
| T26 | COMMAND | text editing keys, paste | repeats (C-BARREPEAT) | COMMAND | as today; **web:** a held key repeats in the `:` line, the `/` line and the picker filter (winit/smithay drop the OS's repeats, so the shell repeats it itself at the desktop's rate and delay: `wl_keyboard.repeat_info` on Wayland, XKB controls on X11, `config.rs` KEY_REPEAT_* until known; never modifiers, Enter or Esc; stops on release, focus loss and mode change);  paste as in "Paste" below (a paste is an edit for T45). **web:** Left, Right, Home, End, Backspace, Ctrl-w (delete the word before the cursor), Ctrl-u (delete to the start); Ctrl/Alt/Super chords type nothing |
| T27 | COMMAND | Enter | none | NORMAL | run the command (`:q` may quit) |
| T28 | COMMAND | Esc | none | NORMAL | cancel |

### Completion: ghost suggestion (see "Completion" below)

User, 2026-11-20: completion is a suggestion shown AS YOU TYPE after the cursor, e.g.
`:go https://google|.com` (`|` = cursor, `.com` drawn dimmed: "ghost text"). Tab cycles
the suggestion through the other matches; Enter runs the line WITH the suggestion.
These rows replace T35-T44 (the external menu, DROPPED by the user the same day).

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T45 | COMMAND | an edit (insert, Backspace, paste, Ctrl-w ...) | cursor at the end of the line, the word under it is not empty, its source has 1+ matches | COMMAND | ghost = the rest of the FIRST match in the source's order (history: the C7 ranking); match index 0 |
| T46 | COMMAND | an edit or a cursor move | otherwise (no match, empty word, cursor not at the end) | COMMAND | no ghost |
| T47 | COMMAND | Tab | a ghost is shown, 2+ matches | COMMAND | ghost = the next match (wraps to the first); the typed text does not change |
| T53 | COMMAND | Tab | a ghost is shown, exactly 1 match | COMMAND | like bash: the match is selected, i.e. the ghost becomes typed text; after a command name or a `:set` argument a space follows (`:se` Tab -> `:set `), after a URL none. Then T45 for the new word (user, 2026-11-20) |
| T48 | COMMAND | Shift-Tab | a ghost is shown, 2+ matches | COMMAND | ghost = the previous match (wraps) |
| T49 | COMMAND | Tab / Shift-Tab | no ghost | COMMAND | nothing |
| T50 | COMMAND | Enter | a ghost is shown | NORMAL | run the typed text + the ghost (`:go https://google.com/`) |
| T51 | COMMAND | Right / End | a ghost is shown | COMMAND | the ghost becomes typed text (keep typing after it); then T45 for the longer word (user, 2026-11-20) |
| T52 | COMMAND | Delete | a ghost is shown | COMMAND | hide the ghost until the next edit, so Enter runs only what was typed (user, 2026-11-20). Needed to go to a URL that is a prefix of a history entry |

T35-T44 are DROPPED (user, 2026-11-20): T35-T38 and T42-T44 were the external dmenu-style
menu (tofi) and its MENU state, T39-T41 the `o` key. The ghost rows above replace them.
Numbers are not reused.

### Events that are not keys

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T29 | NORMAL, G, SEARCH, COMMAND, PICK | an editable element gains focus because of the user (a click, a key the shell gave the page, `gi`), or a click opens a popup (select menu, context menu) | the focus comes within `CLICK_FOCUS_MS` (1000) of that mouse press, of an Enter given to the page (T19; T18's find + Enter, timed from the real press: Chromium moves the focus at `find-clear`), or of `gi` (T111) | INPUT | the click goes to the page as usual; an open SEARCH/COMMAND line or the picker (T68) is cancelled first. A click on a button that makes the page focus a field (a "Reply" opening a textarea) counts. **Changed 2026-11-24 (user, C-AUTOFOCUS):** "moving to INPUT mode when input elements get focus confuses me [...] I'm thinking of not changing modes on autofocus, but should make sure that once I hit i, the autofocused input gets focus"; "include gi too". The page's own focus is T32. A cross-document load start ends the window (a field the new document autofocuses is the page's, even right after the click that loaded it). **web:** the host reports `editable-focused true` after the engine moved focus (for every focus that lands in an editable, also from one field straight to another); the shell decides with `Keys::editable_focused`; `editable-focused false` (blur) never changes the mode. A `<select>` is not editable, so a new popup (xdg_popup) within `CLICK_FOCUS_MS` (1000) of a mouse press also switches (it is the select's menu, or a context menu, whose keys must reach the page); that mode returns when the popup closes (T57) |
| T30 | any | other mouse input | none | same | goes to the page as today |
| T31 | any | navigation (any cause: `:go`, `h`/`l`, a followed link or click, page script) | none | same | highlights and the current match cleared; the kept query stays (`n` on the new page searches again); an open SEARCH/COMMAND line stays open. **web:** cleared on the host's `load start` of a cross-document load (the host drops its session itself and the shell forgets the index and `[i/N]`); same-document navigations (a `#fragment`, `history.pushState`) get no load callbacks, so the host's `load start ... same-document` is the signal: the shell sends `find-clear` and forgets the index and `[i/N]` (C5-FRAG). A `find-result` is matched to its `find` by id; one nobody asked for (CEF's stale `0 0` at a load start) is ignored (C5-STALE) |
| T32 | NORMAL, G, SEARCH, COMMAND, PICK | page script focuses an element (`el.focus()`, `autofocus`), or a background buffer's page focuses one | the element is editable and the user did not cause it (T29's window) | same | **changed 2026-11-24 (user, C-AUTOFOCUS):** "moving to INPUT mode when input elements get focus confuses me [...] I'm thinking of not changing modes on autofocus, but should make sure that once I hit i, the autofocused input gets focus"; "include gi too". The mode stays (an open line or the picker is no longer cancelled), and the shell asks the host to blur the field and remember it ("parked", `blur-field`), so NORMAL's scroll keys, which are real arrow/PageDown keys to whatever has focus (T13), scroll the page; `i` focuses it again (T6). Refocus guard: at most one blur per buffer per `CLICK_FOCUS_MS`; a page that takes the focus back after that keeps it (and NORMAL stays). The blur runs the page's own blur handlers (seen live: a "required" validation message appears; suggestion lists would close). In INPUT nothing happens (the user is typing; the page may move its focus). (2026-11-23, replaced: any editable focus switched to INPUT.) No auto-return on blur: a blur (`editable-focused false`), focus of a non-editable element and a popup the page opens without a user click change nothing. **web:** `Keys::editable_focused` -> `FocusOutcome::Parked` / `Kept`; the buffer's `parked` and `blurred_at` |
| T33 | NORMAL, G, SEARCH, COMMAND, PICK | key release | none | same | not delivered to the page *(proposed, P2)*. **web:** a release goes to the page exactly when its press did: a key pressed in INPUT and released after Esc Esc is released in the page too (no stuck keys); a press in NORMAL is never followed by a release in the page (T34). When the shell window loses the desktop's keyboard focus (C-FOCUSKEYS: smithay's winit backend drops winit's synthetic releases) the shell releases, in the page, every key whose press it delivered and has no release for, and forgets what it saw held; a physical release that comes after the refocus is then not delivered (T33 still holds). The held modifiers are the desktop's (C3-SHIFTSYNC): its `wl_keyboard.modifiers` (X11: XKB StateNotify) set the seat's modifier state while no key is down, so a Shift held at focus-in or set by a modifiers event alone still makes `;` a `:` and `g` a `G`; the page is told of it only in INPUT |
| T34 | INPUT | release of a key whose press was NOT delivered to the page (the `i` of T6, the second Esc of T2) | none | INPUT | not delivered either, so the page never sees a `keyup` without its `keydown` |

## Accesskey (QWPT7c)

**web:** in INPUT the chord goes to the engine, which implements accesskey itself. In
NORMAL it is not forwarded (T21) unless a `config.rs` row says otherwise. scratchy's
text below is kept as the record of its own implementation.

Alt+Shift+<key> (Ctrl and Super not held) activates the element whose `accesskey` lists that character, in
both modes (`input_host::accesskey_chord`, before the mode's own handling; no match = the key goes on to the
mode as before). Chosen: the Firefox convention on Linux/Windows, and what WPT's `pressAccessKey` sends (Shift+Alt;
Chrome uses plain Alt). Plain Alt+key is left to the window manager and menu mnemonics, and Alt+Shift
is never part of a vim binding, so every NORMAL-mode key keeps working. A button/link/checkbox/radio/label gets
focus and a click with no pointer or mouse events (client coordinates 0); a text field, textarea or select only
takes focus.

## Search (S1-S7)

**web:** the engine searches and paints the highlights (CEF `CefBrowserHost::Find`,
WebKitGTK `WebKitFindController`); the shell keeps the keys, the query and the status-bar
messages (T15-T25). Built by C5 (smartcase: the shell sends `match_case` when the query has a
capital; CEF paints all matches and the active one in its own colours and scrolls the match
into view itself, S4, but how it centres is the engine's). What carries over unchanged: smartcase, no regex, the wrap messages,
`[i/N]`. What depends on the engine: what counts as a match (rendered text, inline
boundaries, iframes) and how a match is scrolled into view (S4). **web:** Enter on a match
is T18: the shell ends the find and the page gets the Enter; S7 below is scratchy's and is
not built.

- **What is searched:** the rendered text of the laid-out top-level document (the
  layout's text runs), not DOM text. Hidden text never matches; every match already has
  rectangles to scroll to and paint. A match may cross inline boundaries (`foo<b>bar</b>`
  matches "foobar"), not block boundaries. Plain text, smartcase (case-insensitive unless
  the query has a capital letter). No regex. Not inside iframes or text fields.
- **Highlights:** chrome painted over the page, like the scrollbar. Every match one
  colour, the current match a stronger one. Recomputed after every relayout; the current
  match stays the nearest one to where it was.
- **Bringing the screen to a match (S4):** on T15 and T23, if the match is not fully
  visible the page scrolls so it is vertically centred (clamped at the page ends); if it
  is fully visible nothing scrolls. A match inside a scrollable box scrolls that box
  first, then the page.
- **Click (S7; scratchy only, replaced in web by T18, C-ENTER):** a real click, through the same path as a mouse click (page click
  handlers, then the default action), never a direct navigation to an `href`, so a site's
  own router (YouTube, GitHub) handles its links. Target: let L be the set of distinct
  nearest `a[href]` / `area[href]` ancestors of the match's text runs. L empty: click the
  element holding the match's first character. L has one link: click the element holding
  the first matched character inside that link (handlers on inner spans run and the event
  bubbles to the link). L has two or more: T19. There is no "on screen" requirement
  (user 2026-11-20).
- **Not now** (each only on request): `?` backward search, `*`/`#`, regex, highlighting
  while typing, search in iframes or text fields.

## Status bar

Always visible at the bottom, and it takes its own height: the page's viewport (layout
height, `innerHeight`, maximum scroll, hit-testing, the JS thread's viewport) is the
window minus the bar. **web:** the compositor sizes the host window to the shell window
minus the bar; the page never overlaps it. It shows the mode (or the SEARCH/COMMAND line being typed), the
current URL and the last message (`[i/N]`, wrap messages, T16/T17 messages,
errors). Other fields (load progress, hovered link target, pending `g`) only on request.
C-BELL: `[N!]` before the buffer number counts the buffers with unread notifications, and the bar
flashes for a notification ("Notifications").

## Completion

In COMMAND, the word under the cursor gets a **ghost suggestion** as you type (T45-T52):
the rest of the best match, drawn dimmed after the cursor; Tab / Shift-Tab cycle it
through the other matches, Enter runs the line with it. Where the matches come from is
set per command, like bash's `complete`:

- **The first word** completes from the command names (built in).
- **Later words** ask the command's **source**, from the `COMPLETERS` table in
  `src/config.rs`: one row per command, either a built-in Rust function or an external
  program. No row: no completion for that command.
  ```rust
  pub(crate) const COMPLETERS: &[(&str, Source)] = &[
      ("set", Source::Builtin(complete_set_args)),
      ("go",  Source::Builtin(complete_history_urls)),
      // ("go", Source::Cmd(&["my-url-source"])),
  ];
  ```
  **web:** scratchy's `:set js`/`motion` (engine settings) are dropped; `:set` has
  `caret`, `nocaret` and `caret?` (C-CARETCMD, below). The rows are `go` (the bookmarks, then the history), `bookmarkDelete` (the bookmarks),
  `refresh` (`hard`) and `set` (`config::SET_ARGS`; a selected `:set` argument gets a space,
  T53). The commands are `:go`, `:refresh [hard]`, `:historyBack`, `:historyFront`, `:historySearch` (the picker, C-PICK), `:bookmarkAdd`, `:bookmarkDelete [url]`, `:bookmarkSearch` (C-BOOKMARKS, "Bookmarks"), `:openExternal [url]` (C-EXTERNAL, "Opening a page in another browser"; completes like `:go`), `:help`,
  `:let`, `:set`, `:q`/`:quit`.
  `:go` without a scheme means `https://` (scratchy's QCOMP6); the ghost still continues
  exactly what was typed. C3-GOHTTP: when the run line's URL (typed + accepted ghost) equals
  a history entry's URL with `https://`/`http://` and `www.` stripped, `:go` puts that
  entry's own scheme before the word, so a ghost taken from an `http://` entry typed without its scheme stays
  `http://` (`:go 127` Tab Enter -> `http://127.0.0.1:8000/`). A typed `http://`/`https://`
  always wins; with no history match `https://` is assumed.
- **An external source** is run with no shell (argv as written) and gets the context
  in its environment, like bash's `COMP_LINE`: `SCRATCHY_LINE` (the whole line),
  `SCRATCHY_WORD` (the word being completed), `SCRATCHY_ARG` (its index, 1 = first
  argument), `SCRATCHY_URL` (the current page), `SCRATCHY_STATE_DIR`. **web:** the
  names become `WEB_LINE`, `WEB_WORD`, `WEB_ARG`, `WEB_URL`, `WEB_STATE_DIR`. It prints one
  candidate per line on stdout. Non-zero exit or nothing printed = no candidates.
- **Matching and the ghost text.** A candidate matches when the typed word is a prefix
  of it. For `:go` both sides are compared with `https://`/`http://` and `www.` removed,
  and the ghost is the candidate's rest after that point, so it always continues exactly
  what was typed: typed `https://google`, candidate `https://www.google.com/`, ghost
  `.com/`, Enter runs `:go https://google.com/`.
- **Per keystroke cost.** Built-in sources answer from memory: the history is read and
  ranked once when the `:go` argument is first edited and kept while the line is open.
  An external source runs once per line and argument position, on a thread; its output
  is kept and filtered by scratchy per keystroke (it is not re-run per key).
- **The built-in `:go` source** (`complete_history_urls`) reads the history file and
  ranks it (`STASH.md` "History" / "Ranking"): site roots by visits to the site, then
  pages by their own visits, ties by recency. That order is the ghost order: the first
  match is shown, Tab walks down the list. It keeps the entries whose URL, with
  `https://`/`http://` and `www.` stripped, starts with the word. No fuzzy matching.
- **No menu** (user, 2026-11-20): an external dmenu-style menu (tofi) was planned and
  DROPPED in favour of the ghost; there is no `MENU_CMD`.
- **Safety:** nothing from a page or the history is put into an argv or a shell; an
  external source gets its context only in environment variables, and runs without a
  shell.
- External sources run on a thread; scratchy keeps drawing and handling events while
  they run, and the result arrives as an `AppEvent`. A result that arrives after its
  command line was closed, or for another argument position, is dropped.

## Paste

**web:** built (C6). The `:` and `/` lines read the desktop's clipboard directly (the shell
window is a client of the desktop: smithay-clipboard on Wayland, x11-clipboard on X11). In
INPUT the paste keys go to the page; just before they do, the shell offers the page the
desktop's clipboard if it changed since it was last carried across (same before a middle
click, for the primary selection). An unreadable clipboard changes nothing and says nothing.

Every status-bar mode you type into (today COMMAND and SEARCH; user, 2026-11-20) takes a
paste: Ctrl-v, Ctrl-Shift-v (terminal habit) and Shift-Insert insert the system
clipboard's text at the cursor. Newlines, tabs and other control characters in the
pasted text become a space and trailing whitespace is dropped (a copied URL often ends
in a newline, and the bar is one line). An empty or unreadable clipboard changes
nothing. The clipboard is the Wayland one (scratchy is a Wayland window; item QPASTE1).
**web:** the shell is both a client of the host display (whose clipboard is "the system
clipboard") and the compositor of the engine host; it bridges the two, so a copy in the
page reaches the host clipboard and a paste in the page reads it (C6, text only; the
primary selection is bridged too: a selection in the page becomes the desktop's primary.
Middle-click paste inside the page does nothing: CEF does not do it, even for its own
selection).

## Registers

Vim-style registers (user 2026-11-21: "copy the URL", keeping vim parity for the
later page cursor and visual mode). Like vim, copying goes through a register, and the
system clipboard is the register `+`.

| Register | Read | Write |
|---|---|---|
| `%` | the current URL, exactly what the status bar shows (never asks the page, so it works with JS off) | no: `scratchy: register @% is read-only` |
| `+` | the system clipboard (Wayland, as in "Paste") | yes |

**web (C3, C6):** `%` is the URL of the host's last `url` event. `+` is the desktop's
clipboard (C6), read and written directly by the shell, so `:let @+ = @%` and `yy` work in
any mode the page is in; `@+` is readable (`:let @+ = @+` says `web: the clipboard is
empty` for an empty one). Messages start with `web:` instead of `scratchy:` (`Copied to
clipboard` stays). `:let @+ = <JS expression>` (QREG2) is not carried (usage line). `*`
(primary) is not a register yet (its own item, as in scratchy).

- `:let @<dst> = @<src>` copies a register into another (spaces optional), as vim's
  `:let @+ = @%` copies the file name. The status bar says `Copied to clipboard`, or
  `scratchy: no URL to copy` (a stdin start page), `scratchy: no register @x`,
  `scratchy: the clipboard is empty`, `scratchy: could not reach the system clipboard`,
  or the usage line for anything else.
- `yy` in NORMAL is `:let @+ = @%` (T54-T56). Once the page cursor exists, vim parity
  would make `yy` "yank the cursor's line"; the user chose `yy` = URL anyway (2026-11-21),
  to be revisited with the cursor design (QCURSOR-D).
- After QREG1, each its own item: `:let @+ = <JS expression>` (QREG2), `*` (primary selection),
  `"`/`0`/`a`-`z`, `/` (the kept query), `:registers`.

## Configuration

Everything a user may want to change lives in **one file, `src/config.rs`**, in the dwm
`config.h` spirit: edit, rebuild (user, 2026-11-20). That includes the key bindings
(today the `match` in `resolve_binding`), `ESC_ESC_MS`, `COMPLETERS`, the
history settings and the scroll steps. Moving the existing knobs there is its own
refactoring item (QCONF1, no behaviour change) and comes first; every later item adds
its knobs there. Exception (user, 2026-11-20): the tracker block/allow lists stay in
QCFG1's separate runtime file, updated independently of the code.

**2026-11-24 (user):** a profile's own commands, key mappings and start buffers come from
`init.js` scripts (global and per profile), not config.rs: SCRIPTING.md. config.rs keeps
web's own defaults; a user mapping takes precedence over a built-in binding.

## Caret browsing (experiment, C-CARETCMD)

User, 2026-11-23: "Expose caret browsing from a command mode command, so I can experiment
with it but don't change the defaults." `:set caret` turns on Chromium's own caret browsing
(F7 in Chrome; host command `caret on`, PROTOCOL.md), `:set nocaret` turns it off, `:set
caret?` says `caret` or `nocaret`. Off at every start, not kept between runs (the host turns
the engine preference off when it starts). It takes effect at once, no reload. The NORMAL
bindings are unchanged, so with caret on they move the caret instead of scrolling, because
they send real keys (measured with `c_enter_live.py`):

- `j`/`k`, Down/Up: caret one line down/up; the page scrolls only when the caret leaves the
  screen. A caret moved into a link focuses the link (Enter then follows it, T19).
- Left/Right: caret one character.
- `gg`/Home and `G`/End: start/end of the caret's LINE, not of the page (no scrolling).
- Ctrl-d/Ctrl-u, Space, PageDown/PageUp: scroll the page; the caret stays where it was.
- `/word` then Esc (T7) puts the caret at the match; `/word` then Enter (T18) also does, but
  gives the page an Enter.
- A text field: `j` moves the caret past an `<input>` without focusing it (no
  `editable-focused`, the mode stays NORMAL). A click into a field still focuses it (T29).
- Nothing in the shell draws the caret; Chromium draws it as a text caret.

The rest of caret mode (our own caret, C5-ENTER) is parked.

## Picker (C-PICK)

Built by C-PICK (`shell/src/picker.rs`). Decided 2026-11-23 (DECISIONS.md "A picker, bookmarks, buffers
as tabs, popups like Chrome"); the rows below are the implementer's, from the user's words in that
decision ("a list the shell draws over the page, above the status bar", "fuzzy filtering, the same keys
in every list (move, Enter runs the row, Esc closes) and a mode of its own").

- A list the shell draws over the page, right above the status bar; the page is not resized. Opened only by
  dedicated commands: `:historySearch` (built), `:bookmarkSearch` (C-BOOKMARKS), `:bufferList`/`:ls` (C4),
  `:popupSearch` (C-POPUPS). Context-menu actions (C-CMDS, parked) may become a source later.
- It is a mode of its own, **PICK**: the status bar shows `>` and the filter being typed (with a caret, like
  the `:` line); the list's header shows `PICK`, the source's name and `matches/total`; the page gets no keys.
- Each row is a label plus a dimmed detail; what is matched is the source's choice (history and bookmarks:
  URL and title; buffers: title, hostname, description). It never shows Tab candidates: the ghost
  (T45-T53) stays the only completion of the `:` line.
- **Matching** (the same in every list, `picker::fuzzy_score`): fzf-style. Each whitespace-separated term
  of the filter must appear in one of the row's fields with its letters in order (not necessarily
  together); smartcase as in Search. Score: every matched letter counts, a match at a word start (after a
  non-alphanumeric character, or the text's start) or a camelCase hump scores more, a run of consecutive
  matches keeps the bonus of its first letter, a gap between matches costs; the best placement of the term
  is found. A row's score is the sum over the terms (each in its best field). **Best first**, ties in the
  source's own order (history: the `:go` ranking, STASH.md "Ranking"). An empty filter lists everything in
  the source's order.
- **Drawn** at the window's scale like the bar (C6b): painted at the framebuffer's resolution with the bar's
  font and paddings; at most `PICKER_MAX_ROWS` (12) rows of `PICKER_ROW_H` (22) logical px under a header row
  of the same height, fewer when the page is shorter; the list scrolls to keep the selection shown. Colours
  and sizes are `config.rs` `PICKER_*`.
- **`:historySearch [filter]`**: the history file (STASH.md "History") in the `:go` ranking (site roots
  first, then pages). Label: the newest title recorded for exactly that URL (a site root without one: its
  host; a page without one: its URL, and no detail); detail: the URL. Matched on the URL without
  `https://`/`http://`/`www.`, and the title. Enter loads the URL in the current page (`open`). An argument
  is typed into the filter *(proposed)*.

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T58 | COMMAND | Enter on `:historySearch [filter]` (also `:bookmarkSearch`, `:bufferList`, `:popupSearch`) | the source has rows | PICK | the list opens with every row matching the filter (none typed: all, in the source's order), the first selected. No rows: stays NORMAL, status bar `web: the history is empty` |
| T59 | PICK | text editing keys, paste | repeats (C-BARREPEAT) | PICK | edit the filter (the `:` line's keys, T26: Left, Right, Home, End, Backspace, Ctrl-w, Ctrl-u, paste); after a change the list is re-ranked (fuzzy, best first) and the best match selected. The page gets no keys, `i` and `j` are typed |
| T60 | PICK | Ctrl-n, Down | not on the last row | PICK | the next row is selected; the list scrolls to keep it shown. On the last row nothing happens (no wrap) |
| T61 | PICK | Ctrl-p, Up | not on the first row | PICK | the previous row is selected (no wrap) |
| T61a | PICK | PageDown, PageUp (keypad too) | none | PICK | the selection moves a screenful (`PICKER_MAX_ROWS` rows) down or up, stopping at the last or first row (no wrap); the list scrolls to keep it shown *(C-PICK-PAGE)* |
| T62 | PICK | Enter (keypad Enter too) | a row matches | NORMAL | the picker closes, then the selected row's action runs (history: `open <url>` in the current page) |
| T63 | PICK | Enter | no row matches | PICK | nothing: edit the filter or Esc |
| T64 | PICK | Esc | none | NORMAL | the picker closes; nothing runs |
| T65 | PICK | Tab, Shift-Tab, Delete, other Ctrl/Alt/Super chords | none | PICK | nothing (no Tab candidates in a picker) |
| T66 | PICK | a mouse press | over the list | NORMAL | the picker closes; the press and its release are not delivered to the page (it was under the list) (the rows are not clickable, keyboard only) |
| T67 | PICK | a mouse press | elsewhere (the page outside the list, the bar) | NORMAL | the picker closes; the click goes to the page as usual (as T29 for the `:` line) |
| T68 | PICK | an editable gains focus, or a click opens a popup | none | INPUT | as T29: the picker closes first; a popup's return (T57) is to NORMAL |

**Adding a source** (C-BOOKMARKS, C4, C-POPUPS): a function returning `Vec<picker::Row>` (label, detail, the
fields to match, a `PickAction`) in the source's own order, opened from its command with
`Keys::open_picker(name, rows, filter, empty_message)`; a new kind of action is a `PickAction` variant and its
arm in `Keys::run_pick_action`.

## Bookmarks (C-BOOKMARKS)

Built by C-BOOKMARKS (`shell/src/bookmarks.rs`). The user's decision, 2026-11-23 (DECISIONS.md "A picker,
bookmarks, buffers as tabs, popups like Chrome"): "A plain file `DIR/bookmarks` (`url<TAB>title`, editable by
hand); `:bookmarkAdd`, `:bookmarkDelete [url]`, `:bookmarkSearch`; bookmarks also feed `:go`'s ghost, above
history." The rows below are the implementer's reading of it. The file format is STASH.md "Bookmarks".

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T69 | COMMAND | Enter on `:bookmarkAdd` | the page has a URL (not empty, not `about:blank`) | NORMAL | appends `url<TAB>title` to `DIR/bookmarks` (the title the host reported; none when it is only the URL); status bar `bookmarked: <url>`. No page: `web: no page to bookmark` |
| T70 | COMMAND | Enter on `:bookmarkAdd` | the URL is already bookmarked | NORMAL | the title is replaced, the line keeps its place; `bookmark updated: <url>` |
| T71 | COMMAND | Enter on `:bookmarkDelete [url]` | the URL (the page's when none is given) is bookmarked | NORMAL | the line is removed; `bookmark removed: <url>` |
| T72 | COMMAND | Enter on `:bookmarkDelete [url]` | not bookmarked | NORMAL | nothing changes; `web: not bookmarked: <url>` |
| T73 | COMMAND | Enter on `:bookmarkSearch [filter]` | there are bookmarks | PICK | the picker (T58-T68) over the bookmarks, newest first; label = title (a bookmark without one: its URL), detail = URL; matched on the URL without scheme/`www.` and the title; Enter loads the URL in the current page (T62) |
| T74 | COMMAND | Enter on `:bookmarkSearch` | no bookmarks | NORMAL | `web: no bookmarks (:bookmarkAdd adds the page)` |
| T75 | COMMAND | typing `:go <word>` | a bookmark and a history entry match | COMMAND | the ghost lists the bookmarks (in the order they were added) before the history's ranking (T45-T53, "Completion") |
| T76 | COMMAND | typing `:bookmarkDelete <word>` | a bookmark's URL continues the word | COMMAND | the ghost offers the bookmarked URLs only (same matching as `:go`) |

The file is written as a temp file `DIR/.bookmarks.tmp-<pid>` renamed over `bookmarks`, under the history's lock;
a failed write says `web: bookmark failed: <error>` and leaves the old file. Without a state directory the
commands say `web: no state directory: bookmarks are off`. `--forget-site` does not touch bookmarks.

## Buffers (C4)

Built by C4 (`shell/src/buffers.rs`; host side PROTOCOL.md "Several pages"). Replaces the deferred tabs
(DECISIONS.md "A picker, bookmarks, buffers as tabs, popups like Chrome"). Commands are camelCase with vim
aliases; the ghost offers the long names only, an alias typed in full still runs.

| command | alias | what it does |
|---|---|---|
| `:new [url]` | `:bufferNew` | a new buffer, current at once (the old one is Ctrl-^'s); the URL is `:go`'s (same completion, `https://` assumed, relative forms); no URL: an `about:blank` buffer and the `:` line open with `go ` typed |
| `:bufferList [filter]` | `:ls` | the picker over the buffers: number, title, hostname, description; `!` after `%`/`#` on a buffer with unread notifications, whose detail starts with the latest one's title (C-BELL, T109) |
| `:buffer N`, `:buffer <partial>` | `:b` | switch to buffer N, or to the one whose title, hostname or description matches |
| `:bufferNext`, `:bufferPrev` | `:bn`, `:bp` | the next / previous buffer (wraps) |
| `:bufferKill [N]` | `:bd` | kill the current buffer (or N); killing the last one leaves an empty `about:blank` buffer |
| `:bufferDescribe [text]` | | set the current buffer's description (kept across navigation); no text clears it |
| Ctrl-^ (NORMAL) | | the previously shown buffer |

Every buffer is a page of the one engine host (each page has a window of its own; the host hides every
page but the shown one, so a buffer in the background runs like a background tab in Chrome: no
animation frames, timers at most once a second, audio keeps playing). Buffer numbers are the shell's:
1, 2, 3, ... in the order buffers were opened, never reused in a run (vim's). Per buffer the shell keeps
the URL, the title, whether it is loading or crashed, the search (kept query, find session, `[i/N]`), the
mode a popup returns to (T57) and the description. Not now (the user's decision): searching page
contents, restoring buffers after a restart.

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T77 | COMMAND | Enter on `:bufferList [filter]` / `:ls` | none | PICK | the picker (T58-T68) over the buffers in number order; label `N name` with vim's `%` after the current buffer's number and `#` after the alternate's (name: the title, else the URL); detail: the hostname, then the description; matched on the title, hostname and description; an argument is typed into the filter; Enter shows the buffer (T78) |
| T78 | COMMAND | Enter on `:buffer N` / `:b N` | buffer N exists | NORMAL | buffer N becomes current: the host shows its page (`show-tab`) and hides the others; the bar shows its URL, its last message is gone (a crashed page says so, T89). The current one: `buffer N: <name>`. No such buffer: `web: no buffer N`. `:buffer` alone names the current buffer |
| T79 | COMMAND | Enter on `:buffer <partial>` / `:b <partial>` | not a number | NORMAL | the buffers whose title, hostname or description contains the text (case-insensitive, the words as one string): exactly one becomes current (T78); none: `web: no buffer matches <text>`; several: `web: more than one buffer matches <text> (:bufferList)` (vim's rule) |
| T80 | COMMAND | Enter on `:bufferNext` / `:bn`, `:bufferPrev` / `:bp` | none | NORMAL | the next / previous buffer in number order becomes current, wrapping at the ends; with one buffer nothing changes |
| T81 | COMMAND | Enter on `:bufferKill [N]` / `:bd [N]` | another buffer remains | NORMAL | buffer N (default: the current one) goes and its page is closed (`close-tab`); `buffer N killed`. If it was current, the alternate buffer (Ctrl-^'s) becomes current, else the next one. A buffer killed before its page existed has its page closed when it comes. No such buffer: `web: no buffer N`; a word that is not a number: `web: usage: :bufferKill [N]` |
| T82 | COMMAND | Enter on `:bufferKill` / `:bd` | it is the last buffer | NORMAL | a new buffer with an empty `about:blank` page becomes current first, then the old one goes as in T81 (the user does not have to know it was the last one). The new buffer has no alternate |
| T83 | COMMAND | Enter on `:bufferDescribe [text]` | none | NORMAL | the current buffer's description becomes the text (`buffer N: <text>`); no text clears it (`buffer N: description cleared`). Kept across navigation in that buffer; never written to disk |
| T84 | NORMAL | Ctrl-^ (Ctrl-Shift-6 on a US layout; Ctrl-6 too) | an alternate buffer exists | NORMAL | the buffer shown before the current one becomes current (so Ctrl-^ twice comes back). None: `web: no alternate buffer`. In INPUT the page gets the key |
| T85 | any | the current buffer changes | none | same | the status bar shows `[N]` before the URL (N: the current buffer's number) |
| T86 | any | a second `web <url>` (C-SINGLE) | none | NORMAL | the URL opens in a new buffer (`new-tab`), which becomes current; the bar says `opened <url> (from another web) in buffer N`. An open `:` line or picker is closed |
| T87 | any | switching buffers; an event of a page that is not current | none | same | the URL, title, search (query, find session, `[i/N]`), loading state and popup return (T57) are the buffer's own: `n` in another buffer uses that buffer's query (`No previous search` if it has none), coming back `n` continues where it was. A page in the background updates only its buffer: its `editable-focused` never switches to INPUT, its find results, load errors and titles never reach the bar |
| T88 | COMMAND | `forgot` (after `:forget-site`, `web --forget-site`) | none | same | every buffer whose page is on the forgotten site (the domain or a host under it) reloads, shown or not |
| T89 | any | `crashed` | none | same | only that buffer's page is gone (the others keep running); the buffer stays. Current: as before, `web: the page crashed; :refresh loads it again`; in the background: `web: the page of buffer N crashed (the other buffers are fine)`, and switching to it shows the first message |
| T90 | COMMAND | typing a command name | none | COMMAND | the ghost offers `bufferList`, `buffer`, `bufferNext`, ... but never `ls`, `b`, `bn`, `bp`, `bd`; a word that is an alias in full has no ghost text, so Enter runs it as typed (`:b` stays `:b`, not the first `b...` name); Tab still cycles the long names |
| T91 | COMMAND | Enter on `:q` / `:quit` | not on `:help` | (exit) | the host closes every page (`quit`) and exits when the last one is gone; the shell opens no page meanwhile |
| T92 | any | Ctrl-click, Shift-click, Ctrl+Shift-click on a link | none | same | nothing opens (C-NOTAB stays): no buffer, no page, no navigation, in NORMAL as in INPUT. **Amended 2026-11-24 (C-MIDCLICK, user: "Middle click on links should basically do a :new")**: the middle-click is no longer in this row, see T105. In NORMAL a click with no modifier held also tells the page "none held" first, since a buffer switch (Ctrl-^) left the new page believing Ctrl+Shift was down. **C-NORMALMODS** (user 2026-11-23): in NORMAL a pointer click carries the modifiers actually held (the shell tells the page the held modifiers just before the press and puts them back, unmodified, after the release), so the host's rules apply there too; NORMAL's own keys still reach the page unmodified (`G` is End, T21). A middle-click: see T105. C-POPUPS kept this, see "Popups" |
| T103 | COMMAND | Enter on `:new <url>` / `:bufferNew <url>` | one word | NORMAL | a new buffer (`new-tab` with the URL, as `:go` would resolve it: `https://` assumed, `/x` `./x` `#x` `?x` relative to the page on screen, a history URL minus its scheme keeps that scheme) becomes current (`show-tab` when its page exists); the buffer left becomes the alternate (Ctrl-^ returns). A word `:go` refuses (relative with no page): its message, no buffer. Two or more words: `web: usage: :new [url]`. The argument completes as `:go`'s |
| T105 | NORMAL, INPUT | middle-click on a link (`a[href]`, `area[href]`; no modifier held) | the pointer is over the link | NORMAL | as `:new <url>` (T103) with the link's absolute URL: a new buffer, current at once, the old one the alternate (Ctrl-^ returns); the clicked page does not navigate; the new page has no `window.opener` and no popup blocking applies. From INPUT too: the new buffer starts in NORMAL. A middle-click on anything else does nothing (no paste, DECISIONS.md "No middle-click paste"); a middle-click with Ctrl/Shift/Alt held, a `javascript:` link, or a URL that is not `http:`/`https:`/`file:` opens nothing. The host tells Ctrl+click from middle-click by the DOM event (`auxclick` button 1), not by Chromium's disposition (the same for both) |
| T104 | COMMAND | Enter on `:new` / `:bufferNew` | no argument | COMMAND | as T103 with an empty `about:blank` page, then the `:` line opens again with `go ` already typed (cursor after it), so typing the address and Enter navigates the new buffer (Esc leaves an empty buffer). |

## Popups (C-POPUPS)

Built by C-POPUPS (`shell/src/popups.rs`; host side PROTOCOL.md "Popups"). DECISIONS.md "A picker,
bookmarks, buffers as tabs, popups like Chrome": "a `window.open` from a user gesture opens a new buffer
with `window.opener` kept (OAuth/payment logins work); one without a gesture is blocked, the status bar
says `popup blocked: <url>` (or that it had no URL), and `:popupOpen` (latest) / `:popupSearch` (the
buffer's list, cleared when it navigates away) open it in a new buffer. A `target=_blank` link opens a new
buffer." The engine host is the popup blocker: it tells a click's popup from a script's by the user
gesture.

| command | what it does |
|---|---|
| `:popupOpen` | open the current buffer's latest blocked popup in a new buffer (`window.opener` kept) |
| `:popupSearch [filter]` | the picker over the current buffer's blocked popups; Enter opens one |

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T93 | any | the page calls `window.open` from a user gesture (a click, a key in INPUT) | none | same | a new buffer for the popup (the next number), shown at once: the host shows its page, `buffer N: popup from buffer M` (the popup's first load then clears the message, as any load does). The opener's buffer becomes the alternate (Ctrl-^ goes back). `window.opener` works in the popup, so it can post to its opener or call it (login and payment popups); its size and position features are ignored (every page fills the page area). Not a mode change |
| T94 | any | a click on a `target=_blank` link (or Enter in a `target=_blank` form) | none | same | a new buffer, as T93 (`window.opener` is null in it: Chromium's implicit `noopener`, as in Chrome). The current page is not navigated |
| T95 | any | a popup calls `window.close()` | none | same | only its buffer goes, as `:bufferKill` of it (T81: the alternate, usually the opener, becomes current): `buffer N closed by the page`. The opener is not touched |
| T96 | any | the page calls `window.open` (or opens a `target=_blank` link) without a user gesture (a timer, page load, a script after the click's activation expired) | none | same | blocked: the page's `window.open` returns null, the popup joins the buffer's blocked list and the bar says `popup blocked: <url>` (`popup blocked (no URL)` for `window.open()` with no URL). A blocked popup of a buffer that is not current only joins that buffer's list. The list is cleared when the page starts another document (a link, `:go`, back/forward, `:refresh`); a same-document navigation keeps it |
| T97 | COMMAND | Enter on `:popupOpen` | the current buffer has a blocked popup | NORMAL | the latest one leaves the list (`opening blocked popup: <url>`); the host re-runs the page's `window.open` in the frame that asked (an iframe too) and lets it through, so it opens as T93 with `window.opener` = that frame: `buffer N: blocked popup opened from buffer M`. If that frame is gone the URL loads fresh: `buffer N: blocked popup loaded fresh from buffer M (its frame is gone: no window.opener)`. None: `web: no blocked popup in this buffer`. An argument: `web: usage: :popupOpen` |
| T98 | COMMAND | Enter on `:popupSearch [filter]` | the current buffer has a blocked popup | PICK | the picker (T58-T68) over the buffer's blocked popups, newest first: label the URL (`(no URL)`), detail the target name and `in a frame`; matched on the URL and the name; Enter opens the row's popup as T97 (it leaves the list). None: stays NORMAL, `web: no blocked popup in this buffer` |

Modifier clicks (C1-INPUT's open case, decided here by the user's words "Ctrl/middle-click on links still
open nothing", DECISIONS.md): in INPUT the page gets the modifiers with the keys; in NORMAL (user
2026-11-23, C-NORMALMODS) a click carries the modifiers held (the page is told just before the press and
unmodified again after the release), so Ctrl/Shift/Ctrl+Shift-click are dropped by the
host in both modes (T92); a middle-click on a link opens it as `:new` (T105, amended 2026-11-24). NORMAL's keys are unaffected: the page's modifiers are never changed by a key
there. Not now (DECISIONS.md): a per-site "always allow popups" list.

## Opening a page in another browser (C-EXTERNAL)

User, 2026-11-23, after C-WEBEX (#718) showed Webex needs WebRTC H.264, which the prebuilt CEF lacks: "Let's do both.
Create :openExternal but let's also build chromium" (DECISIONS.md). A cheap way out for sites web cannot play (Webex now, DRM sites).

| ID | Mode | Key | Condition | Next | Action |
|---|---|---|---|---|---|
| T99 | COMMAND | Enter on `:openExternal [url]` | the URL (the buffer's own when none is given; else normalised as `:go`: `https://` assumed, `/x` `./x` `#x` `?x` relative to the page) has the scheme http, https or file | NORMAL | runs the other browser with that URL as ONE argv element and does not wait for it: `opened externally: <url> (<program>)`. The `:` line completes the argument like `:go` (bookmarks, then history) |
| T100 | COMMAND | Enter on `:openExternal [url]` | any other scheme (`javascript:`, `data:`, `about:`, `mailto:`), no page, or no program | NORMAL | nothing is run: `web: :openExternal: refused javascript: URLs (only http, https, file are handed over)`, `web: :openExternal: no page to open`, or `web: :openExternal: <program>: not found (...)`. Two arguments: `web: usage: :openExternal [url]` |
| T101 | - | which program | - | - | `config::EXTERNAL_BROWSER` (an argv; `%s` becomes the URL, else the URL is appended) when set; else `$BROWSER`: its `:`-separated entries (each may carry arguments and `%s`) in order, the first whose program exists; else `xdg-open` |
| T102 | - | how it is started | - | - | never through a shell, so the URL is not interpreted; a new session (`setsid`), stdio on /dev/null, reaped by a thread (no zombie); it outlives web |

## Notifications (C-BELL)

User, 2026-11-24: "We should plan a discreet notification feedback. Maybe a visual bell that causes the
status bar to flash? :ls should show which buffers have unread notifications"; "focus on catching notify
events if possible". Only a page's own Web Notifications count (no title-counter or audio heuristics).
They reach the shell as the host's `notify` event (PROTOCOL.md "Notifications"); no sound, no desktop
popup, ever. Shell side: `shell/src/notify.rs`; knobs in config.rs (`NOTIFY_SITES`, `BELL_*`,
`NOTIFY_UNREAD_MAX`).

| ID | Mode | Event | Condition | Next | Action |
|---|---|---|---|---|---|
| T106 | any | a page notifies (`notify`) | its buffer is not the one shown | same | the notification is kept as unread on that buffer (at most `NOTIFY_UNREAD_MAX`, the oldest go; one with the same non-empty tag as an unread one replaces it); the bar shows `[N!]` before the buffer number, N = the buffers with unread notifications (`[1!] [2] https://...`); the bell (T110). The mode, the `:` line, the picker and the message are untouched |
| T107 | any | a page notifies | its buffer is the one shown | same | the bell only: it is on screen, so nothing becomes unread (also when the shell window is not focused) |
| T108 | any | a buffer is shown (`:buffer`, `:bn`/`:bp`, Ctrl-^, `:ls` Enter, `:bd` moving on, a popup) | it has unread notifications | - | they are read: cleared, the buffer leaves `[N!]`'s count (none left: no `[N!]`) |
| T109 | COMMAND | Enter on `:ls` / `:bufferList` | a buffer has unread notifications | PICK | its label gets `!` after vim's `%`/`#` (`1#! Chat`, `3! Mail`) and its detail starts with the latest unread notification's title, then the hostname and the description |
| T110 | any | the visual bell | a notification of any buffer, and no flash in the last `BELL_MIN_INTERVAL_MS` (1000) | same | the status bar takes `BELL_BG`/`BELL_FG` (the mode colour, inverted) for `BELL_FLASH_MS` (150; 0 = no bell), then its own colours again. Sites: `NOTIFY_SITES` = `All` (default: every page sees permission "granted" without a prompt) or `Only(&[hostnames])` (exact; others see "denied" and send nothing); a change needs a rebuild and restart. Notifications from inside a worker are lost silently |
| T112 | COMMAND | Enter on `:notifications [filter]` (C-NOTIFLIST) | none | PICK | the picker (T58-T68) over this session's notifications, newest first (the rows of every buffer, shown or not): label `N title` (N the buffer's number; `N! title` while unread), detail the body, the notifier's hostname and `(closed)` when the buffer is gone; typing filters on the title, body and hostname. None yet: stays, message `web: no notifications this session`. A notification of the shown buffer is listed already read (T107); showing a buffer reads all of its notifications in the list too (T108). Kept in memory only, at most `NOTIFY_LIST_MAX` (100, config.rs; the oldest go); not saved across runs |
| T113 | PICK | Enter on a `:notifications` row | its buffer still exists | NORMAL | that buffer is shown as `:buffer N` (T77: the old one becomes Ctrl-^'s) and the notification is read, as are the buffer's others (T108) |
| T114 | PICK | Enter on a `:notifications` row | its buffer was killed | NORMAL | the notification is read; the buffer's URL as of the notification (else the notifier's origin) is opened in a new buffer through `:new`'s path (T103), the bar says `buffer N is closed: reopened <url> as buffer M`; with no URL at all only `web: buffer N is closed and the notification has no page URL` |

## What went away in scratchy (QKEYS3)

`:set vim` / `:set novim` (`App::vim_enabled`), the SIGUSR1 toggle
(`AppEvent::ToggleVimMode`) and QCFG1's `vim` config key. Input mode takes no key except
Esc Esc, so there is nothing left to switch off, and Esc Esc cannot be locked out (T2).

## Decision log

- 2026-11-24 (C-NOTIFLIST): `:notifications` (T112-T114). The implementer's choices, not asked: a closed
  buffer's notification reopens the page's URL in a new buffer (the URL of its buffer at the time, else the
  notifier's origin) and says so, rather than doing nothing; read = Enter on it or showing its buffer; the
  list is kept in memory only, capped at 100; no dismiss key and no `click` event to the page (the host's
  `Notification` replacement never fires `onclick`: follow-up C-NOTIFCLICK, a host command `notify-click`).

- 2026-11-24 (user, C-AUTOFOCUS): "moving to INPUT mode when input elements get focus confuses me [...] I'm thinking of not changing modes on autofocus, but should make sure that once I hit i, the autofocused input gets focus"; "include gi too". Only a focus the user caused switches to INPUT (T29:
  a mouse press, an Enter given to the page or `gi` within `CLICK_FOCUS_MS`); the page's own focus is
  parked (T32: blurred by the host, `i` focuses it again, T6); `gi` added (T111); T13's note follows.
  The implementer's choices, not asked: a cross-document load start ends the user-caused window (a
  page that autofocuses right after the click that loaded it stays NORMAL); a background buffer's page
  that focuses a field is parked too; an editable focus in INPUT changes nothing; `gi` with a field
  that already has the focus enters INPUT (no `editable-focused` comes); `gi`'s first field is looked
  for in the main frame only; scroll keys and keys other than Enter never count as given to the page.

- 2026-11-24 (C-BELL): notifications built as "Notifications" says (T106-T110). The implementer's choices,
  not asked: a notification of the buffer on screen is never unread, even when the shell window is not
  focused; `[N!]` counts buffers, not notifications, and comes before `[N]`; no status-bar message for a
  notification (the flash, `[N!]` and `:ls` are the feedback; C-NOTIFLIST will list them); the bell is
  rate-limited to one flash a second and inverts the bar to the mode colour; unread lists are capped at
  20 per buffer and a same-tag notification replaces an unread one; the host also rate-limits (burst 5,
  then one every 2 s per page).

- 2026-11-23 (C-NORMALMODS, the user's words in DECISIONS.md): a pointer click in NORMAL carries the held
  modifiers: the shell sends the page `wl_keyboard.modifiers` with the held state just before a press and
  the unmodified state after the release (INPUT: the desktop's); the seat's own state and NORMAL's keys are
  untouched, so `G` is still End. Ctrl/Shift/Ctrl+Shift-click open nothing in either mode (T92); middle-click on a link is `:new` (T105).

- 2026-11-23 (C-POPUPS): popups built as "Popups" says (T93-T98). The implementer's choices, not asked:
  the shell switches to every popup, also one a background buffer opened; a blocked popup that is
  opened leaves the list (as Chrome's blocked-popup list); `:popupSearch` lists newest first; no hint
  after `popup blocked: <url>` (the decision's exact words); a background buffer's blocked popup is not
  announced in the bar (T87's rule). Modifier clicks in NORMAL stay plain clicks (see "Popups").

- 2026-11-23 (C4): buffers built as "Buffers" says (T77-T92). The implementer's choices, not asked: buffer
  numbers never reused (vim's); `:buffer <partial>` needs exactly one match (vim's) and is a substring match,
  not the picker's fuzzy one; `:bufferKill` of the current buffer goes to the alternate, else the next; the
  bar shows `[N]` before the URL; `:bufferList` marks `%`/`#` as vim's `:ls`; a crash keeps the buffer (the
  page stays, `:refresh` loads it) rather than killing it; a second `web <url>` shows the new buffer at once.

- 2026-11-23 (C-PICK): the picker built as "Picker" says (T58-T68). The implementer's choices, not asked:
  the bar prompt `>`, no wrap at the list's ends (T60/T61), Enter with no match does nothing (T63), a press
  over the list is kept from the page (T66), `:historySearch <filter>` pre-types the filter, fzf-style
  scoring with ties in the source's order, the list's best row at the top under a header.

- 2026-11-23 (user, planning): a picker for dedicated searches (`:historySearch`, `:bookmarkSearch`,
  `:bufferList`), bookmarks, buffers as tabs with camelCase commands and vim aliases, popups like
  Chrome ("Picker", "Buffers" above; DECISIONS.md). The 2026-11-20 "no menu" decision was about `:go`'s
  completion only (user: "that decision was specific about the autocompletion of the :go command"); it
  stays, and the picker never shows Tab candidates.

- 2026-11-23 (user, C-ENTER + C-CARETCMD): "Enter in NORMAL forwards to the page. Expose caret
  browsing from a command mode command, so I can experiment with it but don't change the
  defaults." T18-T20 rewritten: no shell-side click and no `Match spans N links`; the engine
  decides. Found live (C-ENTER): Chromium moves focus to the match's link only when the find
  ENDS (CEF `StopFinding` keeping the selection, what `find-clear` does), not while the find is
  open, so T18 ends the find before the Enter; sent at once, the Enter overtook the focus change
  in 1 of 4 runs, so it goes 50 ms later (10/10 runs). `:set caret` / `nocaret` / `caret?`
  added ("Caret browsing").

- 2026-11-23 (user, C3-RCLICK): "when input elements gain focus, we should automatically switch to Input
  Mode. No auto-return on blur since the exit of the form context isn't that clear." This reverses T32
  ("a page cannot pull the user out of NORMAL"); T32 and T29 rewritten. Popups opened by a click return
  to the previous mode when they close unless a field got focus (T57).

- 2026-11-23 (C3): built in web as described in the **web:** notes. The shell's own
  choices (not asked): Ctrl-d/Ctrl-u scroll a page; a popup right after a click switches to
  INPUT (T29 for selects); releases follow their presses (T33); `:help` is a file the shell
  writes next to its control socket and the host loads; `:q` on it goes back.

- 2026-11-23 (user): web adopts this file from scratchy for the compositor shell
  (DECISIONS.md "Architecture C"). The **web:** notes above are the only changes.

- 2026-11-21 (user): copying the URL goes through vim registers: `:let @+ = @%` and `yy`
  (QREG1). Clicking the status bar to copy and mouse-selecting in the status bar were
  dropped/deferred (plan only, nothing built yet); a keyboard page cursor (hjkl, n/N move it, Enter acts on it, visual
  mode + `y`) is the long-term way to copy text (QCURSOR-D, design first).
- QKEYS-D D1-D8, QSEARCH-D S1-S7: `TODO.txt` (user, 2026-11-20).
- 2026-11-20 (user): completion is per-command sources, built in or scripts (QCOMP-D),
  shown as a ghost suggestion while typing; Tab cycles, one match is selected, Enter
  accepts, Right/End take it, Delete hides it (T45-T53). The dmenu menu and `o` were
  planned and dropped the same day. Configuration stays in code, in `src/config.rs`.
- 2026-11-20: "page mode" is called **INPUT** mode ("input passthrough"); first match
  after a search = first at or below the top of the screen; a match touching 2+ links
  warns and does nothing; this file written as the state machine (user).
