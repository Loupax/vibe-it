//! Vim-style registers (KEYS.md "Registers"), ported from scratchy's `src/registers.rs`
//! (QREG1). `%` is the current URL as the status bar shows it (read-only); `+` is the
//! desktop's clipboard (C6, `clipboard.rs`). `:let @<dst> = @<src>` copies one register into
//! another (a user mapping `web.map("n", "yy", ":let @+ = @%<CR>")` copies the URL). `:yank` (`y`,
//! C-YANK) writes the page's selection to `+`.

use crate::clipboard::{SystemClipboard, Which};

/// A register name `:let` knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Register {
    /// `%`: the current URL (read-only).
    Url,
    /// `+`: the system clipboard (C6).
    Clipboard,
}

pub const LET_USAGE: &str = "web: usage: :let @<dst> = @<src>";
pub const COPIED: &str = "Copied to clipboard";
pub const CLIPBOARD_EMPTY: &str = "web: the clipboard is empty";
pub const CLIPBOARD_UNREACHABLE: &str = "web: could not reach the system clipboard";
/// C-YANK: `:yank` with nothing selected in the page.
pub const NOTHING_SELECTED: &str = "web: nothing selected";

/// The two register names of `:let @dst = @src` (spaces optional); None for another shape.
pub fn parse_let(line: &str) -> Option<(char, char)> {
    let rest = line.trim().strip_prefix('@')?;
    let mut it = rest.chars();
    let dst = it.next()?;
    let rest = it.as_str().trim_start().strip_prefix('=')?.trim().strip_prefix('@')?;
    let mut it = rest.chars();
    let src = it.next()?;
    it.as_str().trim().is_empty().then_some((dst, src))
}

pub fn register(name: char) -> Option<Register> {
    match name {
        '%' => Some(Register::Url),
        '+' => Some(Register::Clipboard),
        _ => None,
    }
}

/// `:let @<dst> = @<src>` with `url` as register `%` and `clipboard` as `+` (None: there is no
/// desktop clipboard): does it and returns what the status bar says.
pub fn let_message(line: &str, url: &str, clipboard: Option<&mut (dyn SystemClipboard + 'static)>) -> String {
    let Some((d, s)) = parse_let(line) else {
        return LET_USAGE.to_string();
    };
    let Some(dst) = register(d) else {
        return format!("web: no register @{d}");
    };
    let Some(src) = register(s) else {
        return format!("web: no register @{s}");
    };
    if dst == Register::Url {
        return "web: register @% is read-only".to_string();
    }
    // dst is `+`: the only writable register.
    if src == Register::Url && url.is_empty() {
        return "web: no URL to copy".to_string();
    }
    let Some(clipboard) = clipboard else {
        return CLIPBOARD_UNREACHABLE.to_string();
    };
    let text = match src {
        Register::Url => url.to_string(),
        Register::Clipboard => match clipboard.read(Which::Clipboard) {
            Ok(t) if t.is_empty() => return CLIPBOARD_EMPTY.to_string(),
            Ok(t) => t,
            Err(_) => return CLIPBOARD_UNREACHABLE.to_string(),
        },
    };
    match clipboard.write(Which::Clipboard, &text) {
        Ok(()) => COPIED.to_string(),
        Err(_) => CLIPBOARD_UNREACHABLE.to_string(),
    }
}

/// C-YANK (`:yank`, `y`): writes the page's selected `text` to `+` (`clipboard`; None: there is no
/// desktop clipboard) as is, newlines kept, and returns what the status bar says.
pub fn yank_message(text: &str, clipboard: Option<&mut (dyn SystemClipboard + 'static)>) -> String {
    if text.is_empty() {
        return NOTHING_SELECTED.to_string();
    }
    let Some(clipboard) = clipboard else {
        return CLIPBOARD_UNREACHABLE.to_string();
    };
    match clipboard.write(Which::Clipboard, text) {
        Ok(()) => COPIED.to_string(),
        Err(_) => CLIPBOARD_UNREACHABLE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `+` only; None = unreachable.
    struct Clip(Option<String>);

    impl SystemClipboard for Clip {
        fn read(&mut self, _: Which) -> Result<String, String> {
            self.0.clone().ok_or_else(|| "unreachable".into())
        }
        fn write(&mut self, which: Which, text: &str) -> Result<(), String> {
            assert_eq!(which, Which::Clipboard);
            match &mut self.0 {
                Some(v) => {
                    *v = text.to_string();
                    Ok(())
                }
                None => Err("unreachable".into()),
            }
        }
    }

    #[test]
    fn yank_message_writes_the_text_as_is() {
        let mut c = Clip(Some("old".into()));
        assert_eq!(yank_message("hello\nworld\n", Some(&mut c)), COPIED);
        assert_eq!(c.0.as_deref(), Some("hello\nworld\n"));
    }

    #[test]
    fn yank_message_empty_text_is_nothing_selected() {
        let mut c = Clip(Some("old".into()));
        assert_eq!(yank_message("", Some(&mut c)), NOTHING_SELECTED);
        assert_eq!(c.0.as_deref(), Some("old"), "clipboard unchanged");
        // Checked before the clipboard: also with none.
        assert_eq!(yank_message("", None), NOTHING_SELECTED);
    }

    #[test]
    fn yank_message_unreachable_clipboard() {
        assert_eq!(yank_message("x", None), CLIPBOARD_UNREACHABLE);
        let mut c = Clip(None);
        assert_eq!(yank_message("x", Some(&mut c)), CLIPBOARD_UNREACHABLE);
    }
}
