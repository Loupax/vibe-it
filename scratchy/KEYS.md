# Keys: input mode, normal mode, search

Status: design, not built. Decisions QKEYS-D, QSEARCH-D and QCOMP-D, items QKEYS1-QKEYS4,
QSEARCH1-QSEARCH3, QCONF1 and QCOMP1-QCOMP5 in `TODO.txt`. **This file is the source of truth**: when the TODO
text and this file disagree, this file wins. Rows marked *(proposed)* are Claude's
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

Two short-lived helper states hold the timing rules: **ESC1** (in INPUT, one Esc seen,
waiting for a second) and **G** (in NORMAL, one `g` seen, waiting for a second). The
status bar shows their parent mode.

- scratchy **starts in INPUT** (for now).
- **Sticky** means a mode lasts until a row below changes it. In particular a
  navigation never changes the mode (`h h h` in NORMAL walks back through history).
- The mouse always goes to the page, in every mode.

```mermaid
stateDiagram-v2
    [*] --> INPUT
    INPUT --> ESC1: Esc (page gets it)
    ESC1 --> NORMAL: Esc within 300 ms (page does not get it)
    ESC1 --> INPUT: other key or too late (page gets the key)
    NORMAL --> INPUT: i, or a click into a text field or select
    NORMAL --> SEARCH: /
    NORMAL --> COMMAND: :
    NORMAL --> G: g
    G --> NORMAL: g = top, other key = handled as in NORMAL
    SEARCH --> NORMAL: Enter = go to the first match, Esc = cancel
    COMMAND --> NORMAL: Enter = run, Esc = cancel
    SEARCH --> INPUT: click into a text field or select
    COMMAND --> INPUT: click into a text field or select
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
| T5 | INPUT | any other key press/release | none | INPUT | today's order: page `keydown`/`keyup` (may cancel) -> open select popup -> focused select -> focused text field -> focused media -> Tab focus -> UA scroll keys *(proposed, P6: arrows, PageUp/PageDown, Space/Shift-Space, Home/End)*. scratchy's bindings are never consulted |

The ESC1 timeout is checked lazily, from the time stamp, when the next key arrives; no
timer is needed. The pure function is `resolve_esc_esc(state, key, repeat, now)`.

### NORMAL and G

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T6 | NORMAL | `i` | none | INPUT | nothing else |
| T7 | NORMAL | Esc | none | NORMAL | clear search highlights (the query is kept). Never changes mode |
| T8 | NORMAL | `/` | none | SEARCH | empty search line |
| T9 | NORMAL | `:` | none | COMMAND | empty command line |
| T10 | NORMAL | `g` | none | G | nothing |
| T11 | G | `g` | none | NORMAL | scroll to the top |
| T12 | G | any other key | none | NORMAL | handle that key as in NORMAL (it may itself be `:`, `/`, `i`, ...) |
| T13 | NORMAL | `j` `k` Ctrl-d Ctrl-u `G` Home End | none | NORMAL | scroll (repeats keep scrolling) |
| T14 | NORMAL | `h` `l` | not a repeat | NORMAL | history back / forward |
| T15 | NORMAL | `n` / `N` | a query exists, it has matches on this page | NORMAL | next / previous match in document order, wrapping with "search hit BOTTOM, continuing at TOP" (or TOP/BOTTOM); bring the screen to it (S4) |
| T16 | NORMAL | `n` / `N` | a query exists, no matches here | NORMAL | status bar: `Pattern not found: <query>` |
| T17 | NORMAL | `n` / `N` | no query yet | NORMAL | status bar: `No previous search` |
| T18 | NORMAL | Enter | a current match whose text runs are inside 0 or 1 links | NORMAL | click (S7) |
| T19 | NORMAL | Enter | the current match's runs are inside 2+ different links | NORMAL | status bar: `Match spans N links`; nothing else |
| T20 | NORMAL | Enter | no current match | NORMAL | nothing |
| T21 | NORMAL | any other key | none | NORMAL | nothing. Not delivered to the page *(proposed, P2)*; that includes Tab and Space, and keys for a focused text field or video |

### SEARCH and COMMAND

| # | From | Event | Guard | To | Effect |
|---|---|---|---|---|---|
| T22 | SEARCH | text editing keys | none | SEARCH | edit the query (`TextEditBuffer`, same as `:`) |
| T23 | SEARCH | Enter | query not empty | NORMAL | the query becomes the kept query; find matches; current = the first match whose top is at or below the top of the screen, in document order (none below: wrap to the first match, with the wrap message); bring the screen to it (S4); `[i/N]` in the status bar. No matches: T16's message |
| T24 | SEARCH | Enter | query empty | NORMAL | same as Esc (T25) |
| T25 | SEARCH | Esc | none | NORMAL | cancel: highlights and the kept query as they were before `/` |
| T26 | COMMAND | text editing keys | none | COMMAND | as today |
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
| T29 | NORMAL, G, SEARCH, COMMAND | user click into a text field or select | none | INPUT | the click goes to the page as usual; an open SEARCH/COMMAND line is cancelled first |
| T30 | any | other mouse input | none | same | goes to the page as today |
| T31 | any | navigation (any cause: `:go`, `h`/`l`, a followed link or click, page script) | none | same | highlights and the current match cleared; the kept query stays (`n` on the new page searches again); an open SEARCH/COMMAND line stays open |
| T32 | any | page script focuses an element (`el.focus()`, `autofocus`) | none | same | the mode does NOT change: a page cannot pull the user out of NORMAL. In NORMAL the focused field gets no keys until the user presses `i` or clicks it |
| T33 | NORMAL, G, SEARCH, COMMAND | key release | none | same | not delivered to the page *(proposed, P2)* |
| T34 | INPUT | release of a key whose press was NOT delivered to the page (the `i` of T6, the second Esc of T2) | none | INPUT | not delivered either, so the page never sees a `keyup` without its `keydown` |

## Search (S1-S7)

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
- **Click (S7, T18):** a real click, through the same path as a mouse click (page click
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
window minus the bar. It shows the mode (or the SEARCH/COMMAND line being typed), the
current URL and the last message (`[i/N]`, wrap messages, T16/T17/T19 messages,
errors). Other fields (load progress, hovered link target, pending `g`) only on request.

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
- **An external source** is run with no shell (argv as written) and gets the context
  in its environment, like bash's `COMP_LINE`: `SCRATCHY_LINE` (the whole line),
  `SCRATCHY_WORD` (the word being completed), `SCRATCHY_ARG` (its index, 1 = first
  argument), `SCRATCHY_URL` (the current page), `SCRATCHY_STATE_DIR`. It prints one
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

## Configuration

Everything a user may want to change lives in **one file, `src/config.rs`**, in the dwm
`config.h` spirit: edit, rebuild (user, 2026-11-20). That includes the key bindings
(today the `match` in `resolve_binding`), `ESC_ESC_MS`, `COMPLETERS`, the
history settings and the scroll steps. Moving the existing knobs there is its own
refactoring item (QCONF1, no behaviour change) and comes first; every later item adds
its knobs there. Exception (user, 2026-11-20): the tracker block/allow lists stay in
QCFG1's separate runtime file, updated independently of the code.

## What goes away *(proposed, P5)*

`:set vim` / `:set novim` (`App::vim_enabled`), the SIGUSR1 toggle
(`AppEvent::ToggleVimMode`) and QCFG1's `vim` config key. Input mode takes no key except
Esc Esc, so there is nothing left to switch off, and Esc Esc cannot be locked out (T2).

## Decision log

- QKEYS-D D1-D8, QSEARCH-D S1-S7: `TODO.txt` (user, 2026-11-20).
- 2026-11-20 (user): completion is per-command sources, built in or scripts (QCOMP-D),
  shown as a ghost suggestion while typing; Tab cycles, one match is selected, Enter
  accepts, Right/End take it, Delete hides it (T45-T53). The dmenu menu and `o` were
  planned and dropped the same day. Configuration stays in code, in `src/config.rs`.
- 2026-11-20: "page mode" is called **INPUT** mode ("input passthrough"); first match
  after a search = first at or below the top of the screen; a match touching 2+ links
  warns and does nothing; this file written as the state machine (user).
