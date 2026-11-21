//! QREG1: vim-style registers, the minimum for copying the URL (KEYS.md "Registers").
//! `:let @+ = @%` copies register `%` (the current URL, read-only) into `+` (the system clipboard).
//! `yy` in normal mode runs the same copy. Only `%` and `+` exist so far.

/// `:let`'s arguments, `@<dst> = @<src>` (spaces optional), parsed into `(dst, src)`. Only the
/// syntax is checked here; whether the registers exist is up to the caller.
pub(crate) fn parse_let(args: &str) -> Result<(char, char), String> {
    let usage = || "scratchy: usage: :let @<register> = @<register>".to_string();
    let (dst, src) = args.split_once('=').ok_or_else(usage)?;
    let reg = |s: &str| {
        let mut c = s.trim().strip_prefix('@')?.chars();
        let r = c.next()?;
        c.next().is_none().then_some(r)
    };
    Ok((reg(dst).ok_or_else(usage)?, reg(src).ok_or_else(usage)?))
}
