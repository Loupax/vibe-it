//! Pairs the engine host's page windows with their `tab-created` events (web/PROTOCOL.md
//! "Pairing"). The two arrive on different sockets, so either can come first.
//!
//! C4, several pages: the host announces every page with `tab-created` before it maps the page's
//! window, and maps windows in the order of their `tab-created`s. So a new toplevel without a parent
//! is the page of the oldest announced tab that has no window yet (FIFO). A parentless toplevel that
//! arrives while no tab is waiting is held as a candidate (shown as a dialog meanwhile): the next
//! `tab-created` takes the oldest candidate (its event was read after the window). A toplevel with a
//! parent is always a dialog.

use std::collections::VecDeque;

#[derive(Debug)]
pub struct Pairing<T> {
    /// Tabs announced whose window has not come yet, oldest first.
    waiting: VecDeque<u64>,
    /// Parentless toplevels that came while no tab was waiting, oldest first.
    candidates: Vec<T>,
    /// Paired page windows and their tabs.
    pages: Vec<(T, u64)>,
}

impl<T: PartialEq + Clone> Default for Pairing<T> {
    fn default() -> Self {
        Pairing {
            waiting: VecDeque::new(),
            candidates: Vec::new(),
            pages: Vec::new(),
        }
    }
}

impl<T: PartialEq + Clone> Pairing<T> {
    /// A new xdg_toplevel. `has_parent`: it was given a parent toplevel. Returns its tab when it
    /// is a page whose `tab-created` already came; None: a dialog, or a candidate for the next
    /// `tab-created` (`is_candidate`).
    pub fn toplevel(&mut self, t: T, has_parent: bool) -> Option<u64> {
        if has_parent {
            return None;
        }
        match self.waiting.pop_front() {
            Some(tab) => {
                self.pages.push((t, tab));
                Some(tab)
            }
            None => {
                self.candidates.push(t);
                None
            }
        }
    }

    /// The host announced a page. Returns the window it pairs with, if that came first.
    pub fn tab_created(&mut self, tab: u64) -> Option<T> {
        if self.pages.iter().any(|(_, x)| *x == tab) || self.waiting.contains(&tab) {
            return None;
        }
        if self.candidates.is_empty() {
            self.waiting.push_back(tab);
            return None;
        }
        let t = self.candidates.remove(0);
        self.pages.push((t.clone(), tab));
        Some(t)
    }

    /// The tab of a paired page window.
    #[cfg(test)]
    pub fn tab_of(&self, t: &T) -> Option<u64> {
        self.pages.iter().find(|(w, _)| w == t).map(|(_, tab)| *tab)
    }

    /// The window of a paired tab.
    #[cfg(test)]
    pub fn window_of(&self, tab: u64) -> Option<&T> {
        self.pages.iter().find(|(_, x)| *x == tab).map(|(w, _)| w)
    }

    /// Whether `t` is waiting for a `tab-created` (no parent, no tab yet).
    pub fn is_candidate(&self, t: &T) -> bool {
        self.candidates.contains(t)
    }

    /// A toplevel went away; returns its tab if it was a page.
    pub fn toplevel_destroyed(&mut self, t: &T) -> Option<u64> {
        self.candidates.retain(|c| c != t);
        let i = self.pages.iter().position(|(w, _)| w == t)?;
        Some(self.pages.remove(i).1)
    }

    /// `tab-closed`: the page is gone (its window goes, or went, separately).
    pub fn tab_closed(&mut self, tab: u64) {
        self.waiting.retain(|x| *x != tab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_first_then_window() {
        let mut p = Pairing::default();
        assert_eq!(p.tab_created(1), None);
        assert_eq!(p.toplevel("w1", false), Some(1));
        assert_eq!(p.tab_of(&"w1"), Some(1));
    }

    #[test]
    fn window_first_then_event() {
        let mut p = Pairing::default();
        assert_eq!(p.toplevel("w1", false), None);
        assert!(p.is_candidate(&"w1"));
        assert_eq!(p.tab_created(7), Some("w1"));
        assert_eq!(p.tab_of(&"w1"), Some(7));
        assert!(!p.is_candidate(&"w1"));
    }

    #[test]
    fn parented_toplevels_are_dialogs() {
        let mut p = Pairing::default();
        p.tab_created(1);
        assert_eq!(p.toplevel("dlg", true), None);
        assert!(!p.is_candidate(&"dlg"));
        assert_eq!(p.toplevel("w1", false), Some(1));
    }

    #[test]
    fn several_pages_pair_in_announcement_order() {
        let mut p = Pairing::default();
        p.tab_created(1);
        p.tab_created(4);
        assert_eq!(p.toplevel("a", false), Some(1));
        assert_eq!(p.toplevel("b", false), Some(4));
        // A third window before its event: the next tab-created takes it.
        assert_eq!(p.toplevel("c", false), None);
        assert_eq!(p.tab_created(9), Some("c"));
        assert_eq!(p.window_of(4), Some(&"b"));
        assert_eq!(p.tab_created(4), None, "a repeated tab-created changes nothing");
    }

    #[test]
    fn a_closed_page_frees_its_window() {
        let mut p = Pairing::default();
        p.tab_created(1);
        p.toplevel("w1", false);
        assert_eq!(p.toplevel_destroyed(&"other"), None);
        assert_eq!(p.toplevel_destroyed(&"w1"), Some(1));
        assert_eq!(p.window_of(1), None);
        // A page closed before its window came: the next window is not paired with it.
        p.tab_created(2);
        p.tab_closed(2);
        assert_eq!(p.toplevel("w3", false), None);
    }
}
