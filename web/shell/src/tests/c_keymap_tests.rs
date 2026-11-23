//! C-KEYMAP: the pure parts of following the desktop's layout.

use crate::keymap::{keymap_name, keymap_text, read_keymap_fd, Latest};
use smithay::input::keyboard::xkb;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::OwnedFd;

fn keymap_for(layout: &str) -> String {
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let km = xkb::Keymap::new_from_names(&ctx, "", "", layout, "", None, xkb::KEYMAP_COMPILE_NO_FLAGS).expect("keymap");
    km.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
}

/// The character a keymap text gives the evdev key `evdev` (no modifiers).
fn char_for(text: &str, evdev: u32) -> String {
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let km = xkb::Keymap::new_from_string(&ctx, text.to_string(), xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS)
        .expect("compile");
    let st = xkb::State::new(&km);
    st.key_get_utf8(xkb::Keycode::new(evdev + 8))
}

const KEY_Q: u32 = 16;
const KEY_Y: u32 = 21;
const KEY_A: u32 = 30;
const KEY_Z: u32 = 44;

fn scratch_file(tag: &str) -> std::fs::File {
    let path = std::env::temp_dir().join(format!("web-keymap-test-{}-{tag}", std::process::id()));
    let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    f
}

#[test]
fn text_is_cut_at_the_nul() {
    assert_eq!(keymap_text(b"xkb_keymap {}\0\0\0").as_deref(), Some("xkb_keymap {}"));
    assert_eq!(keymap_text(b"xkb_keymap {}").as_deref(), Some("xkb_keymap {}"));
    assert_eq!(keymap_text(b"\0junk"), None);
    assert_eq!(keymap_text(b"  \n"), None);
    assert_eq!(keymap_text(&[0xff, 0xfe]), None);
}

#[test]
fn fd_is_read_from_the_start_whatever_its_offset() {
    let mut f = scratch_file("offset");
    f.write_all(b"xkb_keymap { }\0").unwrap();
    // The writer's offset is at the end, as a compositor's shared description would be.
    assert_eq!(f.seek(SeekFrom::Current(0)).unwrap(), 15);
    let fd: OwnedFd = f.into();
    assert_eq!(read_keymap_fd(fd, 15).unwrap(), "xkb_keymap { }");
}

#[test]
fn empty_fd_is_an_error() {
    let fd: OwnedFd = scratch_file("empty").into();
    assert!(read_keymap_fd(fd, 10).is_err());
}

#[test]
fn repeats_are_not_new() {
    let mut l = Latest::default();
    assert!(l.is_new("a"));
    assert!(!l.is_new("a"));
    assert!(l.is_new("b"));
    assert!(l.is_new("a"));
    l.forget();
    assert!(l.is_new("a"));
}

#[test]
fn a_desktop_keymap_text_gives_the_desktops_characters() {
    // What the desktop sends round-trips: de swaps y/z, fr swaps a/q, us is the default.
    let us = keymap_for("us");
    let de = keymap_for("de");
    let fr = keymap_for("fr");
    assert_eq!((char_for(&us, KEY_Y), char_for(&us, KEY_Z)), ("y".into(), "z".into()));
    assert_eq!((char_for(&de, KEY_Y), char_for(&de, KEY_Z)), ("z".into(), "y".into()));
    assert_eq!((char_for(&fr, KEY_A), char_for(&fr, KEY_Q)), ("q".into(), "a".into()));
}

#[test]
fn name_is_read_for_the_log() {
    assert!(keymap_name(&keymap_for("de")).contains("de"));
    assert_eq!(keymap_name("nothing"), "?");
}
