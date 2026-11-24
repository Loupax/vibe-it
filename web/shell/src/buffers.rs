//! C4: buffers, the shell's tabs (KEYS.md "Buffers", DECISIONS.md "A picker, bookmarks, buffers as
//! tabs, popups like Chrome").
//!
//! Every buffer is one page of the engine host (PROTOCOL.md "Several pages": `new-tab`, `show-tab`,
//! `close-tab`). The shell shows one buffer, the current one; the host hides every other page. Buffer
//! numbers are the shell's own, 1, 2, 3, ... in the order buffers were opened, never reused in a run
//! (vim's); the host's page ids (`tab`) are another numbering.
//!
//! Per buffer the shell keeps the URL, the title, whether it is loading or crashed, the search (the
//! kept query and the host's find session, KEYS.md "Search"), the mode a popup returns to (T57) and
//! the user's description. While a buffer is current its URL, title, search and popup return live in
//! `Keys` (where the rest of the shell reads them); switching saves them into the buffer it leaves and
//! loads the ones of the buffer it shows (`Keys::switch_to`).
//!
//! Pure: no Wayland. The compositor (`state.rs`) carries out the `Effect`s and shows the page window of
//! the current buffer's tab (`State::sync_page`).

use crate::keys::{Effect, KeyMode, Keys, SearchState};
use crate::picker::{PickAction, Row};
use crate::protocol::{Command, Event};

/// One buffer.
#[derive(Debug, Clone, Default)]
pub struct Buffer {
    /// The shell's number (status bar, `:buffer N`, `:bufferList`).
    pub num: u32,
    /// The host's page id, once `tab-created` named it.
    pub tab: Option<u64>,
    /// The `new-tab` request still waiting for its `tab-created`.
    pub req: Option<u64>,
    pub url: String,
    pub title: String,
    /// The user's words for it (`:bufferDescribe`); kept across navigation.
    pub description: String,
    /// A main-frame load started and has not ended.
    pub loading: bool,
    /// The page's renderer died (`crashed`); a new load clears it.
    pub crashed: bool,
    /// The search while the buffer is not current (`Keys::search` while it is).
    pub search: SearchState,
    /// T57's mode to return to while the buffer is not current (`Keys::popup_return` while it is).
    pub popup_return: Option<KeyMode>,
    /// C-POPUPS: the page's blocked popups, oldest first; cleared when it starts another document.
    pub blocked: Vec<crate::popups::BlockedPopup>,
    /// C-BELL: notifications that came while the buffer was not shown, oldest first; cleared when
    /// it is shown (`Keys::switch_to`).
    pub unread: Vec<crate::notify::Note>,
    /// C-AUTOFOCUS: the page focused a field itself and the shell had it blurred: `i` focuses it again.
    /// Ends at a cross-document load and at any later editable focus.
    pub parked: bool,
    /// C-AUTOFOCUS: the last `blur-field` sent (the refocus guard: at most one per `CLICK_FOCUS_MS`).
    pub blurred_at: Option<std::time::Instant>,
}

impl Buffer {
    /// The hostname of its URL ("" when it has none: about:blank, data:, ...).
    pub fn host(&self) -> String {
        url::Url::parse(&self.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default()
    }

    /// What `:bufferList` and the messages call it: the title, else the URL, else `[No Name]`.
    pub fn name(&self) -> String {
        if !self.title.is_empty() {
            self.title.clone()
        } else if !self.url.is_empty() {
            self.url.clone()
        } else {
            "[No Name]".to_string()
        }
    }
}

/// What a `tab-created` meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabCreated {
    /// It is buffer `n`'s page.
    Buffer(u32),
    /// Its buffer was killed before the host answered: close the page.
    Doomed,
    /// Nobody asked for it (a newer host's own page): a new buffer `n` was made for it.
    Adopted(u32),
}

/// Every buffer, in number order, and which one is shown.
#[derive(Debug, Clone)]
pub struct Buffers {
    pub list: Vec<Buffer>,
    /// The current buffer's number.
    pub current: u32,
    /// The buffer shown before the current one (Ctrl-^), if it still exists.
    pub alternate: Option<u32>,
    next_num: u32,
    next_req: u64,
    /// `new-tab` requests of buffers killed before their `tab-created` came.
    doomed: Vec<u64>,
}

impl Default for Buffers {
    fn default() -> Self {
        Buffers::new()
    }
}

impl Buffers {
    /// Buffer 1, the host's first page (its tab comes with the first `tab-created`).
    pub fn new() -> Buffers {
        Buffers {
            list: vec![Buffer {
                num: 1,
                ..Default::default()
            }],
            current: 1,
            alternate: None,
            next_num: 2,
            next_req: 1,
            doomed: Vec::new(),
        }
    }

    pub fn get(&self, num: u32) -> Option<&Buffer> {
        self.list.iter().find(|b| b.num == num)
    }

    pub fn get_mut(&mut self, num: u32) -> Option<&mut Buffer> {
        self.list.iter_mut().find(|b| b.num == num)
    }

    pub fn by_tab_mut(&mut self, tab: u64) -> Option<&mut Buffer> {
        self.list.iter_mut().find(|b| b.tab == Some(tab))
    }

    pub fn current(&self) -> &Buffer {
        self.get(self.current).expect("the current buffer exists")
    }

    pub fn current_mut(&mut self) -> &mut Buffer {
        let n = self.current;
        self.get_mut(n).expect("the current buffer exists")
    }

    /// The current buffer's page id, once known.
    pub fn current_tab(&self) -> Option<u64> {
        self.current().tab
    }

    /// The buffer `by` places after `num` in number order, wrapping (`:bufferNext` 1,
    /// `:bufferPrev` -1).
    pub fn step(&self, num: u32, by: i64) -> u32 {
        let n = self.list.len() as i64;
        let i = self.list.iter().position(|b| b.num == num).unwrap_or(0) as i64;
        self.list[(i + by).rem_euclid(n) as usize].num
    }

    /// A new buffer for `url` (its page not asked for yet): its number and the `new-tab` req.
    pub fn open(&mut self, url: &str) -> (u32, u64) {
        let (num, req) = (self.next_num, self.next_req);
        self.next_num += 1;
        self.next_req += 1;
        self.list.push(Buffer {
            num,
            req: Some(req),
            url: url.to_string(),
            ..Default::default()
        });
        (num, req)
    }

    /// Takes buffer `num` out of the list (it must not be the current one). Returns its page to
    /// close, if it has one; a buffer still waiting for its page has its req remembered, so the
    /// page is closed when it comes.
    pub fn remove(&mut self, num: u32) -> Option<u64> {
        let i = self.list.iter().position(|b| b.num == num)?;
        let b = self.list.remove(i);
        if self.alternate == Some(num) {
            self.alternate = None;
        }
        if b.tab.is_none() {
            if let Some(r) = b.req {
                self.doomed.push(r);
            }
        }
        b.tab
    }

    /// The host announced a page.
    pub fn tab_created(&mut self, tab: u64, req: Option<u64>) -> TabCreated {
        if let Some(r) = req {
            if let Some(i) = self.doomed.iter().position(|d| *d == r) {
                self.doomed.remove(i);
                return TabCreated::Doomed;
            }
            if let Some(b) = self.list.iter_mut().find(|b| b.req == Some(r)) {
                b.req = None;
                b.tab = Some(tab);
                return TabCreated::Buffer(b.num);
            }
        }
        // The first page (no req): the first buffer still without a page and without a request.
        if let Some(b) = self.list.iter_mut().find(|b| b.tab.is_none() && b.req.is_none()) {
            b.tab = Some(tab);
            return TabCreated::Buffer(b.num);
        }
        TabCreated::Adopted(self.adopt(tab))
    }

    /// A new buffer for page `tab`, which the host created by itself (a popup, a newer host's own
    /// page): its number.
    pub fn adopt(&mut self, tab: u64) -> u32 {
        let num = self.next_num;
        self.next_num += 1;
        self.list.push(Buffer {
            num,
            tab: Some(tab),
            ..Default::default()
        });
        num
    }

    /// The buffers whose title, hostname or description holds `text` (case-insensitive).
    pub fn matching(&self, text: &str) -> Vec<u32> {
        let t = text.to_lowercase();
        self.list
            .iter()
            .filter(|b| {
                [b.title.as_str(), b.host().as_str(), b.description.as_str()]
                    .iter()
                    .any(|f| f.to_lowercase().contains(&t))
            })
            .map(|b| b.num)
            .collect()
    }
}

/// `:bufferList`'s rows, in number order: label `N name` (`%` after the number of the current
/// buffer, `#` after the alternate, as vim's `:ls`; then `!` when it has unread notifications,
/// C-BELL), detail the hostname and the description (with unread notifications: the latest one's
/// title first).
/// Matched on the title, hostname and description ("keep it minimal": no page content).
pub fn rows(b: &Buffers) -> Vec<Row> {
    b.list
        .iter()
        .map(|x| {
            let mark = if x.num == b.current {
                "%"
            } else if b.alternate == Some(x.num) {
                "#"
            } else {
                ""
            };
            let bang = if x.unread.is_empty() { "" } else { "!" };
            let host = x.host();
            let latest = x.unread.last().map(|n| n.title.as_str()).unwrap_or("");
            let detail = [latest, host.as_str(), x.description.as_str()]
                .iter()
                .filter(|f| !f.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("  ");
            Row {
                label: format!("{}{mark}{bang} {}", x.num, x.name()),
                detail,
                fields: vec![x.title.clone(), host, x.description.clone()],
                action: PickAction::Buffer(x.num),
            }
        })
        .collect()
}

/// `:bufferKill`'s usage line.
pub const NEW_USAGE: &str = "web: usage: :new [url]";
pub const KILL_USAGE: &str = "web: usage: :bufferKill [N]";

impl Keys {
    /// A page event's effect on its buffer: for every buffer whether it is loading or crashed; for a
    /// buffer that is not current also its URL, title and find session (T31: a cross-document load
    /// ends it). Returns true when the event is not the current buffer's: then the status bar, the
    /// search and the mode must not see it (an `editable-focused` in the background never switches
    /// to INPUT). An event of a page with no buffer (killed, its `tab-closed` still to come) is
    /// dropped the same way.
    pub fn page_event(&mut self, tab: u64, ev: &Event) -> bool {
        let current = self.buffers.current_tab() == Some(tab);
        let Some(b) = self.buffers.by_tab_mut(tab) else {
            return true;
        };
        match ev {
            Event::Load { phase, kind, .. } if kind != "same-document" => {
                if phase == "start" {
                    b.loading = true;
                    b.crashed = false;
                    // C-POPUPS (T96): another document; its blocked popups go (the host's too).
                    b.blocked.clear();
                    // C-AUTOFOCUS: and its parked field.
                    b.parked = false;
                    b.blurred_at = None;
                    if !current {
                        b.title.clear();
                        b.search.forget_session();
                    }
                } else {
                    b.loading = false;
                }
            }
            Event::Url { url, .. } if !current => b.url = url.clone(),
            Event::Title { title, .. } if !current => b.title = title.clone(),
            Event::Crashed { .. } => {
                b.crashed = true;
                b.loading = false;
                if !current {
                    let n = b.num;
                    self.message = format!("web: the page of buffer {n} crashed (the other buffers are fine)");
                }
            }
            _ => {}
        }
        !current
    }

    /// The buffers showing `domain` or a host under it (`:forget-site` reloads them): number, page,
    /// hostname.
    pub fn buffers_on_site(&mut self, domain: &str) -> Vec<(u32, u64, String)> {
        self.save_current();
        let under = |h: &str| !domain.is_empty() && (h == domain || h.ends_with(&format!(".{domain}")));
        self.buffers
            .list
            .iter()
            .filter_map(|b| Some((b.num, b.tab?, b.host().to_ascii_lowercase())))
            .filter(|(_, _, h)| under(h))
            .collect()
    }

    /// Copies the live per-buffer state (`url`, `title`, `search`, `popup_return`) into the current
    /// buffer, so lists and switches see it.
    pub fn save_current(&mut self) {
        let (url, title, search, ret) = (self.url.clone(), self.title.clone(), self.search.clone(), self.popup_return);
        let b = self.buffers.current_mut();
        b.url = url;
        b.title = title;
        b.search = search;
        b.popup_return = ret;
    }

    /// Makes buffer `num` current (it must exist): the state of the one left is saved, the new one's
    /// loaded, and the host is told to show its page (once it has one: else on its `tab-created`).
    pub fn switch_to(&mut self, num: u32) {
        if num == self.buffers.current {
            self.save_current();
            self.message = format!("buffer {num}: {}", self.buffers.current().name());
            return;
        }
        self.save_current();
        self.buffers.alternate = Some(self.buffers.current);
        self.buffers.current = num;
        // C-BELL: shown, so its notifications are read.
        self.buffers.current_mut().unread.clear();
        let b = self.buffers.current().clone();
        self.url = b.url;
        self.title = b.title;
        self.search = b.search;
        self.popup_return = b.popup_return;
        self.message = if b.crashed {
            "web: the page crashed; :refresh loads it again".to_string()
        } else {
            String::new()
        };
        if let Some(tab) = b.tab {
            self.effects.push(Effect::Send(Command::ShowTab { tab }));
        }
    }

    /// Opens `url` in a new buffer and shows it (a second `web <url>`; the last buffer killed).
    pub fn open_buffer(&mut self, url: &str) -> u32 {
        let (num, req) = self.buffers.open(url);
        let u = (!url.is_empty() && url != "about:blank").then(|| url.to_string());
        self.effects.push(Effect::Send(Command::NewTab { req, url: u }));
        self.switch_to(num);
        num
    }

    /// `:new [url]` / `:bufferNew [url]` (C-NEWBUF): a new buffer, current at once (the old one becomes
    /// the alternate). The argument is `:go`'s; none: an `about:blank` buffer and the `:` line open with
    /// `go ` typed, so the address comes straight away.
    pub fn buffer_new(&mut self, args: &[&str]) {
        match args {
            [] => {
                self.open_buffer("about:blank");
                self.open_line_with("go ");
            }
            [word] => match self.go_target(word) {
                Ok(url) => {
                    self.open_buffer(&url);
                }
                Err(m) => self.message = m,
            },
            _ => self.message = NEW_USAGE.to_string(),
        }
    }

    /// `:buffer N` / `:buffer <partial>` (`:b`).
    pub fn buffer_command(&mut self, args: &[&str]) {
        if args.is_empty() {
            self.save_current();
            let b = self.buffers.current();
            self.message = format!("buffer {}: {}", b.num, b.name());
            return;
        }
        let text = args.join(" ");
        if let Ok(n) = text.parse::<u32>() {
            match self.buffers.get(n) {
                Some(_) => self.switch_to(n),
                None => self.message = format!("web: no buffer {n}"),
            }
            return;
        }
        self.save_current();
        match self.buffers.matching(&text)[..] {
            [n] => self.switch_to(n),
            [] => self.message = format!("web: no buffer matches {text}"),
            _ => self.message = format!("web: more than one buffer matches {text} (:bufferList)"),
        }
    }

    /// `:bufferNext` / `:bufferPrev` (`:bn`, `:bp`): wraps; one buffer stays.
    pub fn buffer_step(&mut self, by: i64) {
        let n = self.buffers.step(self.buffers.current, by);
        self.switch_to(n);
    }

    /// Ctrl-^: the buffer shown before this one.
    pub fn buffer_alternate(&mut self) {
        match self.buffers.alternate.filter(|a| self.buffers.get(*a).is_some()) {
            Some(a) => self.switch_to(a),
            None => self.message = "web: no alternate buffer".to_string(),
        }
    }

    /// `:bufferKill [N]` (`:bd`): the buffer goes, its page is closed. The current one is replaced by
    /// the alternate (else the next); the last one by a new `about:blank` buffer.
    pub fn buffer_kill(&mut self, args: &[&str]) {
        let num = match args {
            [] => self.buffers.current,
            [n] => match n.parse::<u32>() {
                Ok(n) if self.buffers.get(n).is_some() => n,
                Ok(n) => {
                    self.message = format!("web: no buffer {n}");
                    return;
                }
                Err(_) => {
                    self.message = KILL_USAGE.to_string();
                    return;
                }
            },
            _ => {
                self.message = KILL_USAGE.to_string();
                return;
            }
        };
        if self.buffers.list.len() == 1 {
            self.open_buffer("about:blank");
            self.buffers.alternate = None;
        } else if num == self.buffers.current {
            let next = self
                .buffers
                .alternate
                .filter(|a| *a != num && self.buffers.get(*a).is_some())
                .unwrap_or_else(|| self.buffers.step(num, 1));
            self.switch_to(next);
        }
        if let Some(tab) = self.buffers.remove(num) {
            self.effects.push(Effect::Send(Command::CloseTab { tab }));
        }
        self.message = format!("buffer {num} killed");
    }

    /// The host closed a page by itself (not through `:bufferKill`): its buffer goes as if killed,
    /// without a `close-tab`.
    pub fn tab_gone(&mut self, tab: u64) {
        let Some(num) = self.buffers.list.iter().find(|b| b.tab == Some(tab)).map(|b| b.num) else {
            return;
        };
        if let Some(b) = self.buffers.get_mut(num) {
            b.tab = None;
            b.req = None;
        }
        self.buffer_kill(&[&num.to_string()]);
        self.message = format!("buffer {num} closed by the page");
    }

    /// `:bufferDescribe [text]`: the current buffer's description; no text clears it.
    pub fn buffer_describe(&mut self, args: &[&str]) {
        let text = args.join(" ");
        let b = self.buffers.current_mut();
        b.description = text.clone();
        let n = b.num;
        self.message = if text.is_empty() {
            format!("buffer {n}: description cleared")
        } else {
            format!("buffer {n}: {text}")
        };
    }

    /// `:bufferList` (`:ls`) [filter]: the picker over the buffers.
    pub fn buffer_list(&mut self, filter: &str) {
        self.save_current();
        let rows = rows(&self.buffers);
        self.open_picker("buffers", rows, filter, "web: no buffers");
    }
}
