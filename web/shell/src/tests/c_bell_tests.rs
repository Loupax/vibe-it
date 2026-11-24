//! C-BELL (KEYS.md "Notifications", T106-T110). Each test names its row.

use std::time::{Duration, Instant};

use crate::buffers::{rows, TabCreated};
use crate::config::{NotifySites, BELL_BG, BELL_FLASH_MS, BELL_MIN_INTERVAL_MS, NOTIFY_UNREAD_MAX, STATUS_BAR_BG};
use crate::keys::*;
use crate::notify::{add_unread, Bell, Note};

/// Buffer 1 (tab 1) and buffer 2 (tab 2), buffer 2 shown (buffer 1 the alternate).
fn two_buffers() -> Keys {
    let mut k = Keys::new(None, String::new(), None);
    assert_eq!(k.buffers.tab_created(1, None), TabCreated::Buffer(1));
    k.url = "https://chat.example.com/".into();
    k.title = "Chat".into();
    k.open_buffer("https://b.test/");
    let req = 1;
    assert_eq!(k.buffers.tab_created(2, Some(req)), TabCreated::Buffer(2));
    k.effects.clear();
    k
}

fn note(title: &str, tag: &str) -> Note {
    Note {
        title: title.into(),
        body: "body".into(),
        tag: tag.into(),
        origin: "https://chat.example.com".into(),
    }
}

#[test]
fn c_bell_t106_a_hidden_buffers_notification_is_unread_and_counted_in_the_bar() {
    let mut k = two_buffers();
    assert_eq!(k.bar_view().url, "[2] https://b.test/");
    assert!(k.notified(1, note("New message", "")));
    assert_eq!(k.buffers.get(1).unwrap().unread.len(), 1);
    assert_eq!(k.unread_buffers(), 1);
    assert_eq!(k.bar_view().url, "[1!] [2] https://b.test/");
    assert!(k.notified(1, note("Another", "")));
    assert_eq!(
        k.bar_view().url,
        "[1!] [2] https://b.test/",
        "buffers are counted, not notifications"
    );
    assert!(!k.notified(9, note("x", "")), "no buffer has that page: nothing, no bell");
}

#[test]
fn c_bell_t107_the_shown_buffers_notification_is_not_unread() {
    let mut k = two_buffers();
    assert!(k.notified(2, note("Here", "")), "still a bell");
    assert!(k.buffers.get(2).unwrap().unread.is_empty());
    assert_eq!(k.unread_buffers(), 0);
    assert_eq!(k.bar_view().url, "[2] https://b.test/");
}

#[test]
fn c_bell_t108_showing_a_buffer_reads_its_notifications() {
    let mut k = two_buffers();
    k.notified(1, note("New message", ""));
    k.buffer_alternate();
    assert_eq!(k.buffers.current, 1);
    assert!(k.buffers.get(1).unwrap().unread.is_empty());
    assert_eq!(k.bar_view().url, "[1] https://chat.example.com/");
    // Any way of showing it: `:buffer N` too.
    k.notified(2, note("x", ""));
    assert_eq!(k.unread_buffers(), 1);
    k.buffer_command(&["2"]);
    assert_eq!(k.unread_buffers(), 0);
}

#[test]
fn c_bell_t109_ls_marks_unread_buffers_and_shows_the_latest_title() {
    let mut k = two_buffers();
    k.notified(1, note("First", "a"));
    k.notified(1, note("Latest", "b"));
    k.save_current();
    let r = rows(&k.buffers);
    assert_eq!(r[0].label, "1#! Chat");
    assert_eq!(r[0].detail, "Latest  chat.example.com");
    assert!(r[1].label.starts_with("2% "), "{}", r[1].label);
    assert!(!r[1].label.contains('!'));
    // With a description: title, host, description.
    k.buffers.get_mut(1).unwrap().description = "work".into();
    assert_eq!(rows(&k.buffers)[0].detail, "Latest  chat.example.com  work");
    // Read: the mark and the title go.
    k.buffer_alternate();
    let r = rows(&k.buffers);
    assert_eq!(r[0].label, "1% Chat");
    assert_eq!(r[0].detail, "chat.example.com  work");
}

#[test]
fn c_bell_t109_same_tag_replaces_an_unread_one_and_the_list_is_capped() {
    let mut l = Vec::new();
    add_unread(&mut l, note("a", "t"));
    add_unread(&mut l, note("b", ""));
    add_unread(&mut l, note("c", "t"));
    add_unread(&mut l, note("d", ""));
    let titles: Vec<_> = l.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(titles, ["b", "c", "d"], "c replaced a (same tag), untagged ones stay");
    for i in 0..NOTIFY_UNREAD_MAX + 3 {
        add_unread(&mut l, note(&i.to_string(), ""));
    }
    assert_eq!(l.len(), NOTIFY_UNREAD_MAX);
    assert_eq!(l.last().unwrap().title, (NOTIFY_UNREAD_MAX + 2).to_string(), "the newest kept");
}

#[test]
fn c_bell_t110_the_bell_flashes_then_is_rate_limited() {
    let mut b = Bell::default();
    let t0 = Instant::now();
    assert_eq!(b.ring(t0), Some(Duration::from_millis(BELL_FLASH_MS)));
    assert!(b.lit);
    b.done();
    assert!(!b.lit);
    let soon = t0 + Duration::from_millis(BELL_MIN_INTERVAL_MS - 1);
    assert_eq!(b.ring(soon), None, "within the interval: no flash");
    assert!(!b.lit);
    let later = t0 + Duration::from_millis(BELL_MIN_INTERVAL_MS);
    assert!(b.ring(later).is_some(), "the interval counts from the last flash");
}

#[test]
fn c_bell_t110_a_lit_bell_inverts_the_bar() {
    let mut k = two_buffers();
    assert!(!k.bar_view().flash);
    k.bell.ring(Instant::now());
    let v = k.bar_view();
    assert!(v.flash);
    let mut p = crate::statusbar::Painter::new();
    let mut corner = |v: &BarView| {
        // The right padding: no text there, only the background.
        let c = p.paint(v, 200, 24).unwrap().pixel(199, 0).unwrap();
        (c.red(), c.green(), c.blue())
    };
    assert_eq!(corner(&v), BELL_BG);
    k.bell.done();
    assert_eq!(corner(&k.bar_view()), STATUS_BAR_BG);
}

#[test]
fn c_bell_t110_notify_sites_reach_the_host_as_an_option() {
    use crate::host::notify_sites_arg;
    assert_eq!(notify_sites_arg(NotifySites::All), None, "every site: the host's default");
    assert_eq!(
        notify_sites_arg(NotifySites::Only(&["chat.example.com", "mail.example.com"])).as_deref(),
        Some("--notify-sites=chat.example.com,mail.example.com")
    );
    assert_eq!(notify_sites_arg(NotifySites::Only(&[])).as_deref(), Some("--notify-sites="));
}
