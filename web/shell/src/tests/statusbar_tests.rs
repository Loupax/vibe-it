//! KEYS.md "Status bar" (C3): the layout (ported from scratchy's status_layout tests) and
//! the drawing. Font-independent: relations only (scratchy's R33 rule).

use crate::keys::BarView;
use crate::statusbar::*;

fn lay(avail: f32, mode: &str, url: &str, msg: &str) -> StatusLayout {
    status_layout(avail, 10.0, 80.0, mode, url, msg, &mut |t: &str| t.chars().count() as f32 * 8.0)
}

#[test]
fn status_bar_short_message_fully_shown() {
    let l = lay(
        800.0,
        "NORMAL",
        "https://example.com/a/very/long/path/to/something/else.html",
        "web: load failed",
    );
    assert_eq!(l.message, "web: load failed");
    assert_eq!(l.mode, "NORMAL");
}

#[test]
fn status_bar_long_url_elided_in_the_middle() {
    let url = "https://example.com/a/very/long/path/to/something/else/index.html";
    let l = lay(500.0, "NORMAL", url, "Copied to clipboard");
    assert!(
        l.url.starts_with("https://exa") && l.url.ends_with("index.html") && l.url.contains('\u{2026}'),
        "{}",
        l.url
    );
    let total = (l.mode.len() + l.url.chars().count() + l.message.len()) as f32 * 8.0 + 20.0;
    assert!(total <= 500.0);
}

#[test]
fn status_bar_very_long_message_truncated_only_when_it_alone_exceeds_the_bar() {
    let msg = "m".repeat(200);
    let l = lay(400.0, "NORMAL", "https://example.com/", &msg);
    assert!(l.message.ends_with('\u{2026}') && l.message.chars().count() < 200);
    let ok = "n".repeat(25);
    assert_eq!(lay(400.0, "NORMAL", "https://example.com/", &ok).message, ok);
    assert_eq!(lay(600.0, "NORMAL", "https://x.test/", "").url, "https://x.test/");
}

fn view(mode: &str, line: Option<&str>, ghost: &str) -> BarView {
    BarView {
        mode: mode.into(),
        url: "https://example.com/".into(),
        message: "a message".into(),
        line: line.map(|l| (':', l.to_string(), l.len(), None)),
        ghost: ghost.into(),
        flash: false,
    }
}

/// Pixels that differ from the background, per column.
fn ink_columns(pm: &tiny_skia::Pixmap) -> Vec<bool> {
    let bg = pm.pixel(0, 0).unwrap();
    (0..pm.width())
        .map(|x| (0..pm.height()).any(|y| pm.pixel(x, y).unwrap() != bg))
        .collect()
}

#[test]
fn status_bar_paints_the_mode_url_and_message_and_the_line_with_a_caret() {
    let mut p = Painter::new();
    let (w, h) = (800, crate::config::STATUS_BAR_H as u32);
    let normal = p.paint(&view("NORMAL", None, ""), w, h).unwrap();
    assert_eq!((normal.width(), normal.height()), (w, h));
    let bg = crate::config::STATUS_BAR_BG;
    let px = normal.pixel(w - 1, 0).unwrap();
    assert_eq!(
        (px.red(), px.green(), px.blue(), px.alpha()),
        (bg.0, bg.1, bg.2, 255),
        "an opaque bar"
    );
    // The `:` line always has its caret, even with no font.
    let line = p.paint(&view("NORMAL", Some(""), ""), w, h).unwrap();
    assert!(ink_columns(&line).iter().any(|&c| c), "the caret");
    if !p.has_fonts() {
        eprintln!("no fonts on this machine: text checks skipped");
        return;
    }
    let cols = ink_columns(&normal);
    assert!(cols[..w as usize / 4].iter().any(|&c| c), "mode and URL on the left");
    assert!(cols[w as usize * 3 / 4..].iter().any(|&c| c), "the message on the right");
    let input = p.paint(&view("INPUT", None, ""), w, h).unwrap();
    assert_ne!(normal.data(), input.data(), "the text follows the mode");
    // The ghost is drawn after the typed text, dimmed.
    let typed = p.paint(&view("NORMAL", Some("go exa"), ""), w, h).unwrap();
    let ghosted = p.paint(&view("NORMAL", Some("go exa"), "mple.com/"), w, h).unwrap();
    let last = |pm: &tiny_skia::Pixmap| ink_columns(pm).iter().rposition(|&c| c).unwrap();
    assert!(last(&ghosted) > last(&typed) + 20, "ghost text extends the line");
    assert!(p.measure("NORMAL") > p.measure("INPUT") * 0.5);
}
