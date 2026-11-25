//! C-POPUPS: popups like Chrome (KEYS.md "Popups", T93-T99; DECISIONS.md "A picker, bookmarks,
//! buffers as tabs, popups like Chrome").
//!
//! The engine host is the popup blocker (PROTOCOL.md "Popups"). A popup the page opened with a user
//! gesture (`window.open`, a `target=_blank` link or form) arrives as a `tab-created` with an
//! `opener`: it becomes a new buffer and is shown. A popup without a gesture arrives as
//! `popup-blocked`: it is kept in its buffer's blocked list (cleared when that page starts another
//! document) and the bar says so; `:popupOpen` opens the latest, `:popupSearch` picks one, through
//! the host's `popup-open` (which answers with a `tab-created`, `cause` `popup-open` or
//! `popup-fresh`). A popup that closes itself (`window.close()`) is a page the host closed:
//! `Keys::tab_gone` kills its buffer.
//!
//! Pure: no Wayland (`state.rs` feeds the events and carries out the effects).

use crate::keys::{Effect, Keys};
use crate::picker::{PickAction, Row};
use crate::protocol::Command;

/// A popup the host blocked, as its buffer keeps it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockedPopup {
    /// The host's number for it (`popup-open`).
    pub popup: u32,
    /// "" when it had no URL (`window.open()`).
    pub url: String,
    /// The target name the page gave (`window.open(url, name)`).
    pub name: String,
    /// An iframe asked, not the page itself.
    pub in_frame: bool,
}

/// What the bar says when a popup is blocked (T96).
pub fn blocked_message(url: &str) -> String {
    if url.is_empty() {
        "popup blocked (no URL)".to_string()
    } else {
        format!("popup blocked: {url}")
    }
}

/// `:popupOpen`'s and `:popupSearch`'s message when the buffer has no blocked popup.
pub const NONE_BLOCKED: &str = "web: no blocked popup in this buffer";

/// `:popupSearch`'s rows: newest first (the one `:popupOpen` would open at the top). Label: the
/// URL (`(no URL)`); detail: the target name and `in a frame` when an iframe asked; matched on the
/// URL and the name.
pub fn rows(blocked: &[BlockedPopup]) -> Vec<Row> {
    blocked
        .iter()
        .rev()
        .map(|b| {
            let mut detail = Vec::new();
            if !b.name.is_empty() {
                detail.push(format!("name {}", b.name));
            }
            if b.in_frame {
                detail.push("in a frame".to_string());
            }
            Row {
                label: if b.url.is_empty() { "(no URL)".to_string() } else { b.url.clone() },
                detail: detail.join(", "),
                fields: vec![b.url.clone(), b.name.clone()],
                action: PickAction::Popup(b.popup),
            }
        })
        .collect()
}

impl Keys {
    /// `popup-blocked` of page `tab`: kept in that page's buffer. Returns the bar's message when the
    /// buffer is the current one (a background buffer's blocked popup only joins its list).
    pub fn popup_blocked(&mut self, tab: u64, popup: BlockedPopup) -> Option<String> {
        let current = self.buffers.current_tab() == Some(tab);
        let b = self.buffers.by_tab_mut(tab)?;
        let msg = blocked_message(&popup.url);
        b.blocked.push(popup);
        current.then_some(msg)
    }

    /// The host created a page for a popup of page `opener` (`tab-created` with `opener`): a new
    /// buffer, shown at once (T93, T94, T97). Returns its number.
    pub fn popup_created(&mut self, tab: u64, opener: u64, cause: &str) -> u32 {
        let num = self.buffers.adopt(tab);
        let from = self.buffers.list.iter().find(|b| b.tab == Some(opener)).map(|b| b.num);
        self.switch_to(num);
        let from = from.map(|n| format!(" from buffer {n}")).unwrap_or_default();
        self.message = match cause {
            "popup-open" => format!("buffer {num}: blocked popup opened{from}"),
            "popup-fresh" => format!("buffer {num}: blocked popup loaded fresh{from} (its frame is gone: no window.opener)"),
            _ => format!("buffer {num}: popup{from}"),
        };
        num
    }

    /// Takes blocked popup `n` (None: the latest) out of the current buffer's list and asks the host
    /// to open it.
    fn open_blocked(&mut self, n: Option<u32>) {
        let Some(tab) = self.buffers.current_tab() else {
            self.fail(NONE_BLOCKED.to_string());
            return;
        };
        let list = &mut self.buffers.current_mut().blocked;
        let i = match n {
            None => list.len().checked_sub(1),
            Some(n) => list.iter().position(|b| b.popup == n),
        };
        let Some(i) = i else {
            self.fail(NONE_BLOCKED.to_string());
            return;
        };
        let b = list.remove(i);
        self.effects.push(Effect::Send(Command::PopupOpen { tab, popup: b.popup }));
        self.message = format!("opening blocked popup: {}", if b.url.is_empty() { "(no URL)" } else { &b.url });
    }

    /// `:popupOpen` (T97): the current buffer's latest blocked popup.
    pub fn popup_open(&mut self, args: &[&str]) {
        if !args.is_empty() {
            self.fail("web: usage: :popupOpen".to_string());
            return;
        }
        self.open_blocked(None);
    }

    /// Enter on a `:popupSearch` row (T98).
    pub fn popup_open_n(&mut self, n: u32) {
        self.open_blocked(Some(n));
    }

    /// `:popupSearch [filter]` (T98): the picker over the current buffer's blocked popups.
    pub fn popup_search(&mut self, filter: &str) {
        let rows = rows(&self.buffers.current().blocked);
        self.open_picker("popups", rows, filter, NONE_BLOCKED);
    }
}
