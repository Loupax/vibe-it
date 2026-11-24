//! C-BELL: a page's Web Notifications (the host's `notify` event, PROTOCOL.md) become discreet
//! feedback in the shell (KEYS.md "Notifications"): the status bar flashes (the visual bell, any
//! buffer, rate-limited), each buffer keeps its unread notifications until it is shown, the bar counts
//! the buffers with unread ones (`[2!]`) and `:ls` marks them `!` with the latest title. No sound, no
//! desktop popup.
//!
//! Pure: the compositor (`state.rs`) gives the clock and arms the timer that ends a flash.

use std::time::{Duration, Instant};

use crate::config::{BELL_FLASH_MS, BELL_MIN_INTERVAL_MS, NOTIFY_UNREAD_MAX};
use crate::keys::Keys;

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
        let Some(b) = self.buffers.by_tab_mut(tab) else {
            return false;
        };
        if !current {
            add_unread(&mut b.unread, note);
        }
        true
    }

    /// How many buffers have unread notifications (the bar's `[N!]`).
    pub fn unread_buffers(&self) -> usize {
        self.buffers.list.iter().filter(|b| !b.unread.is_empty()).count()
    }
}
