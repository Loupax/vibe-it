//! Vim-style registers (KEYS.md "Registers"), ported from scratchy's `src/registers.rs`
//! (QREG1). `%` is the current URL as the status bar shows it (read-only); `+` is the
//! system clipboard, which needs C6's clipboard bridge: until then every use of it says so.
//! `:let @<dst> = @<src>` copies one register into another; `yy` runs `:let @+ = @%`.

/// A register name `:let` knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Register {
    /// `%`: the current URL (read-only).
    Url,
    /// `+`: the system clipboard (C6).
    Clipboard,
}

pub const LET_USAGE: &str = "web: usage: :let @<dst> = @<src>";
pub const CLIPBOARD_NOT_YET: &str = "web: @+ (and yy) need the clipboard, not built yet (web/TODO.txt C6)";

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

/// `:let @<dst> = @<src>` with `url` as register `%`: what the status bar says.
pub fn let_message(line: &str, url: &str) -> String {
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
    CLIPBOARD_NOT_YET.to_string()
}
