//! C-CLOSE-POLITE (issue #757): `tab-closed` while the shell is closing, and the exit code.

use crate::buffers::TabCreated;
use crate::keys::*;
use crate::protocol::Command;
use crate::state::exit_code_for;

fn sends(effects: &[Effect]) -> Vec<Command> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(c) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

fn one_buffer() -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.effects.clear();
    k
}

/// Buffer 1 on tab 1, buffer 2 on tab 2 (current).
fn two_buffers() -> Keys {
    let mut k = one_buffer();
    k.open_buffer("https://b.test/");
    assert_eq!(k.buffers.tab_created(2, Some(1)), TabCreated::Buffer(2));
    k.effects.clear();
    k.message.clear();
    k
}

#[test]
fn closing_the_only_buffer_opens_no_new_one() {
    let mut k = one_buffer();
    assert!(!k.tab_closed(1, true));
    assert_eq!(k.buffers.list.len(), 1);
    assert!(sends(&k.effects).is_empty(), "no new-tab while the host exits: {:?}", k.effects);
}

#[test]
fn not_closing_the_only_buffer_is_replaced_as_before() {
    // T82: a page that closed itself while nobody asked leaves a fresh about:blank buffer.
    let mut k = one_buffer();
    assert!(k.tab_closed(1, false));
    assert!(sends(&k.effects).iter().any(|c| matches!(c, Command::NewTab { .. })));
}

#[test]
fn closing_two_buffers_removes_only_the_closed_one() {
    let mut k = two_buffers();
    assert!(k.tab_closed(2, true));
    assert_eq!(k.buffers.list.len(), 1);
    assert_eq!(k.buffers.list[0].tab, Some(1));
    assert_eq!(k.buffers.current, 1);
    let sent = sends(&k.effects);
    assert!(
        sent.iter().all(|c| !matches!(c, Command::NewTab { .. } | Command::CloseTab { .. })),
        "{sent:?}"
    );
    // The other page closes too (the user left it): the last buffer stays, nothing is sent.
    k.effects.clear();
    assert!(!k.tab_closed(1, true));
    assert_eq!(k.buffers.list.len(), 1);
    assert!(sends(&k.effects).is_empty());
}

#[test]
fn a_tab_the_shell_killed_is_not_killed_again() {
    let mut k = two_buffers();
    assert!(!k.tab_closed(99, true));
    assert_eq!(k.buffers.list.len(), 2);
}

#[test]
fn exit_code_is_the_hosts_after_a_close() {
    assert_eq!(exit_code_for(true, Some(0)), 0);
    assert_eq!(exit_code_for(true, Some(5)), 5);
    assert_eq!(exit_code_for(true, None), 1);
    assert_eq!(exit_code_for(false, Some(0)), 1);
    assert_eq!(exit_code_for(false, Some(3)), 3);
    assert_eq!(exit_code_for(false, None), 1);
}
