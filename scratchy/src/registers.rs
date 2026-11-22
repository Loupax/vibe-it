//! QREG1: vim-style registers (KEYS.md "Registers"). `%` is the current URL as the status bar
//! shows it (read-only), `+` is the system clipboard. `:let @<dst> = @<src>` copies one into
//! the other; `yy` runs `:let @+ = @%`.

use crate::App;

/// A register name the `:let` command knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Register {
    /// `%`: the current URL (read-only).
    Url,
    /// `+`: the system clipboard.
    Clipboard,
}

pub(crate) const LET_USAGE: &str = "scratchy: usage: :let @<dst> = @<src>";

/// The two register names of `:let @dst = @src` (spaces optional); None when the line has
/// another shape.
pub(crate) fn parse_let(line: &str) -> Option<(char, char)> {
    let rest = line.trim().strip_prefix('@')?;
    let mut it = rest.chars();
    let dst = it.next()?;
    let rest = it.as_str().trim_start().strip_prefix('=')?.trim().strip_prefix('@')?;
    let mut it = rest.chars();
    let src = it.next()?;
    it.as_str().trim().is_empty().then_some((dst, src))
}

/// `:let @<dst> = <expr>`: the destination and the raw expression (not `@<register>`).
pub(crate) fn parse_let_expr(line: &str) -> Option<(char, &str)> {
    let rest = line.trim().strip_prefix('@')?;
    let mut it = rest.chars();
    let dst = it.next()?;
    let expr = it.as_str().trim_start().strip_prefix('=')?.trim();
    (!expr.is_empty()).then_some((dst, expr))
}

pub(crate) fn register(name: char) -> Option<Register> {
    match name {
        '%' => Some(Register::Url),
        '+' => Some(Register::Clipboard),
        _ => None,
    }
}

impl App {
    /// `:let @<dst> = @<src>`: the status bar says what happened.
    pub(crate) fn let_register(&mut self, line: &str) {
        self.status_message = self.let_message(line);
    }

    fn let_message(&mut self, line: &str) -> String {
        let Some((d, s)) = parse_let(line) else {
            return match parse_let_expr(line) {
                Some((d, expr)) => self.let_expr(d, expr),
                None => LET_USAGE.to_string(),
            };
        };
        let Some(dst) = register(d) else { return format!("scratchy: no register @{d}") };
        let Some(src) = register(s) else { return format!("scratchy: no register @{s}") };
        if dst == Register::Url {
            return "scratchy: register @% is read-only".to_string();
        }
        let text = match src {
            Register::Url => match self.url_register() {
                Some(u) => u,
                None => return "scratchy: no URL to copy".to_string(),
            },
            Register::Clipboard => match crate::forms_text::clipboard_get() {
                Some(t) => t,
                None => return "scratchy: the clipboard is empty".to_string(),
            },
        };
        if crate::forms_text::clipboard_set(&text) {
            "Copied to clipboard".to_string()
        } else {
            "scratchy: could not reach the system clipboard".to_string()
        }
    }

    /// QREG2: evaluates `expr` in the page's main realm and stores `String()` of the result.
    fn let_expr(&mut self, d: char, expr: &str) -> String {
        let Some(dst) = register(d) else { return format!("scratchy: no register @{d}") };
        if dst == Register::Url {
            return "scratchy: register @% is read-only".to_string();
        }
        let Some(js) = self.js.as_ref() else { return "scratchy: :let needs JS (:set js)".to_string() };
        match js.eval_to_string("let", expr) {
            Ok(text) => {
                if crate::forms_text::clipboard_set(&text) {
                    "Copied to clipboard".to_string()
                } else {
                    "scratchy: could not reach the system clipboard".to_string()
                }
            }
            Err(msg) => format!("scratchy: {}", msg.lines().next().unwrap_or("error")),
        }
    }
}
