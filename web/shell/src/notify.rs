//! C-BELL: a page's Web Notifications (the host's `notify` event, PROTOCOL.md) become discreet
//! feedback in the shell (KEYS.md "Notifications"): the status bar flashes (the visual bell, any
//! buffer, rate-limited), each buffer keeps its unread notifications until it is shown, the bar counts
//! the buffers with unread ones (`[2!]`) and `:ls` marks them `!` with the latest title. No sound, no
//! desktop popup.
//!
//! Pure: the compositor (`state.rs`) gives the clock and arms the timer that ends a flash.

use std::time::{Duration, Instant};

use crate::config::{BELL_FLASH_MS, BELL_MIN_INTERVAL_MS, NOTIFY_LIST_MAX, NOTIFY_UNREAD_MAX};
use crate::keys::Keys;
use crate::picker::{PickAction, Row};

/// One notification as the host sent it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Note {
    pub title: String,
    pub body: String,
    /// The page's tag ("" = none): a newer one with the same tag replaces an unread one.
    pub tag: String,
    pub origin: String,
}

/// Adds `note` to a buffer's unread list (oldest first): an unread one with the same non-empty tag
/// is replaced (the page updated it); at most `NOTIFY_UNREAD_MAX` are kept, the oldest go.
pub fn add_unread(list: &mut Vec<Note>, note: Note) {
    if !note.tag.is_empty() {
        list.retain(|n| n.tag != note.tag);
    }
    list.push(note);
    if list.len() > NOTIFY_UNREAD_MAX {
        let extra = list.len() - NOTIFY_UNREAD_MAX;
        list.drain(..extra);
    }
}

/// The visual bell: whether the bar is lit, and the rate limit (a ring within
/// `BELL_MIN_INTERVAL_MS` of the last flash's start does not flash again).
#[derive(Debug, Default, Clone)]
pub struct Bell {
    /// The bar shows the bell's colours.
    pub lit: bool,
    last: Option<Instant>,
}

impl Bell {
    /// A notification came at `now`: Some(how long to stay lit) when the bar flashes, None when the
    /// rate limit keeps it dark (or `BELL_FLASH_MS` is 0: no bell).
    pub fn ring(&mut self, now: Instant) -> Option<Duration> {
        if BELL_FLASH_MS == 0 {
            return None;
        }
        if self
            .last
            .is_some_and(|l| now.saturating_duration_since(l) < Duration::from_millis(BELL_MIN_INTERVAL_MS))
        {
            return None;
        }
        self.last = Some(now);
        self.lit = true;
        Some(Duration::from_millis(BELL_FLASH_MS))
    }

    /// The flash is over (the timer `ring` asked for fired).
    pub fn done(&mut self) {
        self.lit = false;
    }
}

impl Keys {
    /// Page `tab` showed a notification. A buffer that is not current keeps it as unread; the current
    /// one is on screen, so it does not. Returns false when no buffer has that page (killed): no bell.
    pub fn notified(&mut self, tab: u64, note: Note) -> bool {
        let current = self.buffers.current_tab() == Some(tab);
        self.save_current();
        let Some(b) = self.buffers.by_tab_mut(tab) else {
            return false;
        };
        let (num, url) = (b.num, b.url.clone());
        if !current {
            add_unread(&mut b.unread, note.clone());
        }
        // C-NOTIFLIST: the session's list; one of the shown buffer is on screen, so already read.
        self.notices.push(num, url, note, !current);
        true
    }

    /// How many buffers have unread notifications (the bar's `[N!]`).
    pub fn unread_buffers(&self) -> usize {
        self.buffers.list.iter().filter(|b| !b.unread.is_empty()).count()
    }
}

/// C-NOTIFLIST: one entry of `:notifications`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub id: u64,
    /// The number of the buffer whose page showed it.
    pub buffer: u32,
    /// That buffer's URL then (reopened when the buffer is gone), else the notifier's origin.
    pub url: String,
    pub note: Note,
    pub unread: bool,
}

/// This session's notifications, oldest first, at most `NOTIFY_LIST_MAX`.
#[derive(Debug, Default, Clone)]
pub struct Notices {
    pub list: Vec<Notice>,
    next_id: u64,
}

impl Notices {
    pub fn push(&mut self, buffer: u32, url: String, note: Note, unread: bool) {
        self.next_id += 1;
        let url = if url.is_empty() { note.origin.clone() } else { url };
        self.list.push(Notice {
            id: self.next_id,
            buffer,
            url,
            note,
            unread,
        });
        if self.list.len() > NOTIFY_LIST_MAX {
            let extra = self.list.len() - NOTIFY_LIST_MAX;
            self.list.drain(..extra);
        }
    }

    /// Buffer `num` was shown: its notifications are read.
    pub fn read_buffer(&mut self, num: u32) {
        for n in self.list.iter_mut().filter(|n| n.buffer == num) {
            n.unread = false;
        }
    }
}

/// What `:notifications` says with nothing to list.
pub const NONE_YET: &str = "web: no notifications this session";

/// `:notifications`' rows, newest first: label `N title` (`N!` while unread), detail the body, the
/// hostname and `(closed)` when the buffer is gone; matched on the title, body and hostname.
pub fn rows(notices: &Notices, buffers: &crate::buffers::Buffers) -> Vec<Row> {
    notices
        .list
        .iter()
        .rev()
        .map(|n| {
            let host = url::Url::parse(&n.note.origin)
                .ok()
                .or_else(|| url::Url::parse(&n.url).ok())
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_default();
            let closed = if buffers.get(n.buffer).is_none() { "(closed)" } else { "" };
            let bang = if n.unread { "!" } else { "" };
            let detail = [n.note.body.as_str(), host.as_str(), closed]
                .iter()
                .filter(|f| !f.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join("  ");
            Row {
                label: format!("{}{bang} {}", n.buffer, n.note.title),
                detail,
                fields: vec![n.note.title.clone(), n.note.body.clone(), host],
                action: PickAction::Notice(n.id),
            }
        })
        .collect()
}

impl Keys {
    /// `:notifications [filter]` (T112): the picker over this session's notifications.
    pub fn notifications(&mut self, filter: &str) {
        self.save_current();
        let rows = rows(&self.notices, &self.buffers);
        self.open_picker("notifications", rows, filter, NONE_YET);
    }

    /// Enter on a notification (T113): it is read; its buffer is shown, or, when that buffer is gone,
    /// its page is opened in a new one (as `:new <url>`), with a message saying so.
    pub fn notice_open(&mut self, id: u64) {
        let Some(n) = self.notices.list.iter_mut().find(|n| n.id == id) else {
            return;
        };
        n.unread = false;
        let (buffer, url) = (n.buffer, n.url.clone());
        if self.buffers.get(buffer).is_some() {
            self.switch_to(buffer);
            return;
        }
        if url.is_empty() {
            self.fail(format!("web: buffer {buffer} is closed and the notification has no page URL"));
            return;
        }
        let num = self.open_buffer(&url);
        self.notices.read_buffer(buffer);
        self.message = format!("buffer {buffer} is closed: reopened {url} as buffer {num}");
    }
}
