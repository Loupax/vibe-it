//! C-CLIPTYPES: every clipboard type between the desktop and the page on Wayland
//! (`desktop_selection.rs`): the marker type and the types each side is offered, and the shell's
//! own reads of `+` while the desktop holds the page's selection. `shell/test/c6_live.py` checks
//! the bridge itself in the nested desktop.

use crate::clipboard::{Desktop, SystemClipboard, Which};
use crate::desktop_selection::{is_own, mimes_for_desktop, mimes_for_page, summary, MARKER};

fn v(s: &[&str]) -> Vec<String> {
    s.iter().map(|m| m.to_string()).collect()
}

#[test]
fn is_own_finds_the_marker() {
    assert!(is_own(&v(&["text/plain", MARKER])));
    assert!(is_own(&v(&[MARKER])));
    assert!(!is_own(&v(&["text/plain", "image/png"])));
    assert!(!is_own(&[]));
    assert!(!is_own(&v(&["application/x-web-shell-own-not"])));
}

#[test]
fn mimes_for_desktop_adds_the_marker_once_keeps_order_dedups() {
    assert_eq!(
        mimes_for_desktop(&v(&["text/html", "text/plain", "text/html"])),
        v(&["text/html", "text/plain", MARKER])
    );
    assert_eq!(mimes_for_desktop(&v(&["image/png", MARKER])), v(&["image/png", MARKER]));
    assert_eq!(mimes_for_desktop(&[]), v(&[MARKER]));
}

#[test]
fn mimes_for_page_removes_the_marker() {
    assert_eq!(
        mimes_for_page(&v(&["image/png", MARKER, "text/plain", "image/png"])),
        v(&["image/png", "text/plain"])
    );
    assert_eq!(mimes_for_page(&v(&[MARKER])), Vec::<String>::new());
}

#[test]
fn round_trip_never_gives_the_page_the_marker() {
    let page = v(&["text/plain;charset=utf-8", "text/html"]);
    let desktop = mimes_for_desktop(&page);
    assert!(is_own(&desktop));
    assert_eq!(mimes_for_page(&desktop), page);
}

#[test]
fn summary_lists_the_first_few() {
    assert_eq!(summary(&[]), "0 types");
    assert_eq!(summary(&v(&["image/png"])), "1 types: image/png");
    assert_eq!(summary(&v(&["a", "b", "c", "d", "e"])), "5 types: a, b, c, d, ...");
}

/// A Desktop whose clipboard thread is gone: any read that reaches it fails.
#[test]
fn own_selection_is_read_from_memory_without_asking_the_desktop() {
    let mut d = Desktop::unconnected();
    assert!(d.read(Which::Clipboard).is_err(), "not own: the desktop is asked (and is gone)");
    d.set_own(Which::Clipboard, Some("page text".into()));
    assert_eq!(d.read(Which::Clipboard), Ok("page text".into()));
    // The other selection still asks the desktop.
    assert!(d.read(Which::Primary).is_err());
    // A page copy without text: the `:` line gets nothing, not a wait for the desktop.
    d.set_own(Which::Primary, Some(String::new()));
    assert_eq!(d.read(Which::Primary), Ok(String::new()));
    // Clones share it (Keys holds one, the bridge another).
    let c = d.clone();
    c.set_own(Which::Clipboard, None);
    assert!(d.read(Which::Clipboard).is_err());
}

#[test]
fn a_write_ends_the_own_selection() {
    let mut d = Desktop::unconnected();
    d.set_own(Which::Clipboard, Some("page text".into()));
    // `:let @+ = @%`: smithay-clipboard's source replaces the page's on the desktop.
    let _ = d.write(Which::Clipboard, "https://example.org/");
    assert_eq!(d.own(Which::Clipboard), None);
}
