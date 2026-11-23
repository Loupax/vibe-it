//! Pairs the engine host's page window with its `tab-created` event (web/PROTOCOL.md "One
//! page per host"). The two arrive on different sockets, so either can come first.
//!
//! One page only (DECISIONS.md "No tabs yet"): the first toplevel without a parent is the
//! page; every other toplevel (a dialog, a parented window) is shown over it. A second
//! `tab-created` is a protocol error for this shell.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Page,
    Dialog,
}

#[derive(Debug)]
pub struct Pairing<T> {
    page: Option<T>,
    tab: Option<u64>,
    closed: bool,
}

impl<T: PartialEq> Default for Pairing<T> {
    fn default() -> Self {
        Pairing {
            page: None,
            tab: None,
            closed: false,
        }
    }
}

impl<T: PartialEq> Pairing<T> {
    /// A new xdg_toplevel. `has_parent`: it was given a parent toplevel.
    pub fn toplevel(&mut self, t: T, has_parent: bool) -> Role {
        if self.page.is_none() && !has_parent && !self.closed {
            self.page = Some(t);
            Role::Page
        } else {
            Role::Dialog
        }
    }

    /// The host announced its page. Returns whether this completed the pair.
    pub fn tab_created(&mut self, tab: u64) -> Result<bool, String> {
        match self.tab {
            Some(old) => Err(format!("tab-created {tab} after tab-created {old}: one page only (No tabs yet)")),
            None => {
                self.tab = Some(tab);
                Ok(self.page.is_some())
            }
        }
    }

    pub fn is_page(&self, t: &T) -> bool {
        self.page.as_ref() == Some(t)
    }

    /// The page's tab id once both halves are in.
    pub fn paired_tab(&self) -> Option<u64> {
        self.page.as_ref().and(self.tab)
    }

    /// A toplevel went away; true if it was the page.
    pub fn toplevel_destroyed(&mut self, t: &T) -> bool {
        if self.is_page(t) {
            self.page = None;
            self.closed = true;
            true
        } else {
            false
        }
    }

    /// `tab-closed`: the page is gone and the host is exiting.
    pub fn tab_closed(&mut self, tab: u64) -> bool {
        if self.tab == Some(tab) {
            self.closed = true;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_first_then_window() {
        let mut p = Pairing::default();
        assert_eq!(p.tab_created(1), Ok(false));
        assert_eq!(p.paired_tab(), None);
        assert_eq!(p.toplevel("w1", false), Role::Page);
        assert_eq!(p.paired_tab(), Some(1));
    }

    #[test]
    fn window_first_then_event() {
        let mut p = Pairing::default();
        assert_eq!(p.toplevel("w1", false), Role::Page);
        assert_eq!(p.tab_created(7), Ok(true));
        assert_eq!(p.paired_tab(), Some(7));
        assert!(p.is_page(&"w1"));
    }

    #[test]
    fn other_toplevels_are_dialogs() {
        let mut p = Pairing::default();
        assert_eq!(p.toplevel("dlg", true), Role::Dialog);
        assert_eq!(p.toplevel("w1", false), Role::Page);
        assert_eq!(p.toplevel("w2", false), Role::Dialog);
        assert!(p.is_page(&"w1"));
    }

    #[test]
    fn second_tab_created_is_an_error() {
        let mut p: Pairing<&str> = Pairing::default();
        p.tab_created(1).unwrap();
        assert!(p.tab_created(2).unwrap_err().contains("one page only"));
    }

    #[test]
    fn page_gone_is_not_replaced() {
        let mut p = Pairing::default();
        p.toplevel("w1", false);
        p.tab_created(1).unwrap();
        assert!(!p.toplevel_destroyed(&"other"));
        assert!(p.toplevel_destroyed(&"w1"));
        assert_eq!(p.paired_tab(), None);
        // The host is exiting; a late window is not taken as the page.
        assert_eq!(p.toplevel("w3", false), Role::Dialog);
        assert!(p.tab_closed(1));
        assert!(!p.tab_closed(9));
    }
}
