//! C6e: the IME relay (`ime.rs`): the page's text-input-v3 state, and what goes up to the desktop
//! and down to the page, in INPUT only.

use crate::ime::{map_cursor, Commit, Composed, Down, PageState, Relay, TiState, Up};

fn st(text: &str, cursor: i32) -> TiState {
    TiState {
        surrounding: Some((text.into(), cursor, cursor)),
        cause: 0,
        content: Some((4, 0)),
        cursor: Some((10, 20, 0, 22)),
    }
}

/// A relay with the page enabled and the desktop entered.
fn ready() -> Relay {
    let mut r = Relay::default();
    r.desktop_enter();
    r.page_enabled(st("ab", 2));
    r
}

#[test]
fn page_state_enable_commit_disable() {
    let mut p = PageState::default();
    // State before enable is not kept: enable resets it.
    p.surrounding("old".into(), 1, 1);
    p.enable();
    p.content(4, 0);
    assert_eq!(p.commit(), Commit::Enabled);
    assert!(p.enabled);
    assert_eq!(p.current.surrounding, None);
    assert_eq!(p.current.content, Some((4, 0)));
    assert_eq!(p.serial, 1);
    // Later commits change only the fields they set.
    p.surrounding("abc".into(), 3, 3);
    p.cursor(1, 2, 3, 4);
    assert_eq!(p.commit(), Commit::Changed);
    assert_eq!(p.current.surrounding, Some(("abc".into(), 3, 3)));
    assert_eq!(p.current.content, Some((4, 0)));
    assert_eq!(p.current.cursor, Some((1, 2, 3, 4)));
    p.cause(1);
    assert_eq!(p.commit(), Commit::Changed);
    assert_eq!(p.current.cause, 1);
    assert_eq!(p.current.surrounding, Some(("abc".into(), 3, 3)));
    p.disable();
    assert_eq!(p.commit(), Commit::Disabled);
    assert!(!p.enabled);
    // A commit while disabled (and a second disable) does nothing.
    assert_eq!(p.commit(), Commit::Nothing);
    p.disable();
    assert_eq!(p.commit(), Commit::Nothing);
    assert_eq!(p.serial, 6);
}

#[test]
fn enabled_upward_only_in_input_with_page_and_desktop() {
    let mut r = Relay::default();
    // No page text input, no desktop focus: nothing.
    assert_eq!(r.sync(true, (0, 0)), (vec![], vec![]));
    r.page_enabled(st("ab", 2));
    assert_eq!(r.sync(true, (0, 0)).0, vec![], "the desktop has not entered the shell window");
    r.desktop_enter();
    assert_eq!(r.sync(false, (0, 0)).0, vec![], "NORMAL: the shell's keys, no input method");
    let (up, down) = r.sync(true, (5, 7));
    assert_eq!(
        up,
        vec![
            Up::Enable,
            Up::Surrounding("ab".into(), 2, 2),
            Up::Cause(0),
            Up::ContentType(4, 0),
            Up::Cursor(15, 27, 0, 22),
            Up::Commit
        ]
    );
    assert!(down.is_empty());
    assert!(r.up_enabled());
    // Nothing changed: nothing sent (sync runs on every redraw).
    assert_eq!(r.sync(true, (5, 7)), (vec![], vec![]));
}

#[test]
fn page_changes_and_window_moves_go_up() {
    let mut r = ready();
    r.sync(true, (0, 0));
    r.page_changed(st("abc", 3));
    let (up, _) = r.sync(true, (0, 0));
    assert_eq!(up[0], Up::Surrounding("abc".into(), 3, 3));
    assert_eq!(up.last(), Some(&Up::Commit));
    // The page window moved in the shell window: the cursor rectangle follows.
    let (up, _) = r.sync(true, (0, 30));
    assert!(up.contains(&Up::Cursor(10, 50, 0, 22)), "{up:?}");
}

#[test]
fn leaving_input_disables_and_clears_the_preedit() {
    let mut r = ready();
    r.sync(true, (0, 0));
    let down = r.desktop_done(Composed {
        preedit: Some(("ni".into(), 2, 2)),
        ..Default::default()
    });
    assert_eq!(down, vec![Down::Preedit(Some("ni".into()), 2, 2), Down::Done]);
    // Esc Esc: NORMAL.
    let (up, down) = r.sync(false, (0, 0));
    assert_eq!(up, vec![Up::Disable, Up::Commit]);
    assert_eq!(down, vec![Down::Preedit(None, 0, 0), Down::Done]);
    assert!(!r.up_enabled());
    // What the input method still sends is dropped.
    assert!(r
        .desktop_done(Composed {
            commit: Some("x".into()),
            ..Default::default()
        })
        .is_empty());
    // Back to INPUT: enabled again with the page's state.
    assert_eq!(r.sync(true, (0, 0)).0[0], Up::Enable);
}

#[test]
fn leaving_without_a_preedit_tells_the_page_nothing() {
    let mut r = ready();
    r.sync(true, (0, 0));
    let (up, down) = r.sync(false, (0, 0));
    assert_eq!(up, vec![Up::Disable, Up::Commit]);
    assert!(down.is_empty());
}

#[test]
fn desktop_leave_and_page_disable_disable_upward() {
    let mut r = ready();
    r.sync(true, (0, 0));
    r.desktop_leave();
    assert_eq!(r.sync(true, (0, 0)).0, vec![Up::Disable, Up::Commit]);
    r.desktop_enter();
    assert_eq!(r.sync(true, (0, 0)).0[0], Up::Enable);
    r.page_disabled();
    assert_eq!(r.sync(true, (0, 0)).0, vec![Up::Disable, Up::Commit]);
    // A change from a page that is not enabled is ignored.
    r.page_changed(st("zz", 2));
    assert_eq!(r.sync(true, (0, 0)).0, vec![]);
}

#[test]
fn composed_text_goes_down_in_protocol_order() {
    let mut r = ready();
    r.sync(true, (0, 0));
    let down = r.desktop_done(Composed {
        preedit: Some(("x".into(), 1, 1)),
        commit: Some("界".into()),
        delete: Some((3, 0)),
    });
    assert_eq!(
        down,
        vec![
            Down::Delete(3, 0),
            Down::Preedit(Some("x".into()), 1, 1),
            Down::Commit("界".into()),
            Down::Done
        ]
    );
    // A `done` without a preedit clears it; a zero delete is not sent.
    let down = r.desktop_done(Composed {
        commit: Some("a".into()),
        delete: Some((0, 0)),
        ..Default::default()
    });
    assert_eq!(down, vec![Down::Preedit(None, 0, 0), Down::Commit("a".into()), Down::Done]);
}

#[test]
fn cursor_rectangle_maps_by_the_page_origin_only() {
    // Logical on both sides: no scale factor enters (HiDPI: the desktop scales the window).
    assert_eq!(map_cursor(Some((4, 12, 1, 22)), (0, 0)), Some((4, 12, 1, 22)));
    assert_eq!(map_cursor(Some((4, 12, 1, 22)), (100, 50)), Some((104, 62, 1, 22)));
    assert_eq!(map_cursor(None, (100, 50)), None);
}
