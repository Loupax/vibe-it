//! Vim-style registers (KEYS.md "Registers"), ported from scratchy's `src/registers.rs`
//! (QREG1). `%` is the current URL as the status bar shows it (read-only); `+` is the
//! desktop's clipboard (C6, `clipboard.rs`). `:let @<dst> = @<src>` copies one register into
//! another; `yy` runs `:let @+ = @%`.

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
