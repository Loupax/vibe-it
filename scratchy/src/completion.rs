//! QCOMP1: the `:` line's ghost suggestion (KEYS.md "Completion", T45-T53; QCOMP-D C1, C8).
//!
//! Everything here is a pure function over the line text and the cursor: `compute_ghost` finds the
//! word under the cursor, asks its source (the first word: the command names; later words: the
//! command's row in `config::COMPLETERS`) and returns the matches; `Ghost` holds them and the
//! index Tab / Shift-Tab move. `App` only stores the `Ghost`, recomputes it after each edit and
//! draws `Ghost::rest`.

use crate::config::{Source, COMPLETERS, SPACE_AFTER_ARG};

/// What a source is asked: the whole line, the word being completed and its argument index
/// (0 = the command name, 1 = the first argument).
pub(crate) struct CompleteCtx<'a> {
    #[allow(dead_code)] // for sources that look at the whole line (QCOMP2 passes it as SCRATCHY_LINE)
    pub line: &'a str,
    pub word: &'a str,
    #[allow(dead_code)] // for sources with a different list per argument position
    pub arg: usize,
}

/// A live ghost: the candidates for the word that starts at `word_start` (each one a full word
/// that starts with what was typed) and which one is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ghost {
    pub word_start: usize,
    pub candidates: Vec<String>,
    pub index: usize,
    /// Whether selecting the only match is followed by a space (T53: a command name or a `:set`
    /// argument yes, a URL no).
    pub space_after: bool,
}

impl Ghost {
    /// The dimmed text to draw after the cursor and to run on Enter: the shown candidate minus
    /// what is already typed (`text[word_start..]`).
    pub(crate) fn rest(&self, text: &str) -> String {
        let typed = text.get(self.word_start..).unwrap_or("");
        self.candidates[self.index].strip_prefix(typed).unwrap_or("").to_string()
    }

    /// Tab (T47): the next match, wrapping.
    pub(crate) fn next(&mut self) {
        self.index = (self.index + 1) % self.candidates.len();
    }

    /// Shift-Tab (T48): the previous match, wrapping.
    pub(crate) fn prev(&mut self) {
        self.index = (self.index + self.candidates.len() - 1) % self.candidates.len();
    }

    /// Tab with exactly one match (T53) selects it: returns the text to append to the line (the
    /// ghost, plus a space when `space_after`). With 2+ matches it cycles (the typed text does not
    /// change) and returns `None`.
    pub(crate) fn tab(&mut self, text: &str) -> Option<String> {
        if self.candidates.len() == 1 {
            let mut add = self.rest(text);
            if self.space_after {
                add.push(' ');
            }
            Some(add)
        } else {
            self.next();
            None
        }
    }

    /// The line text with the ghost accepted (Enter, T50; Right/End, T51).
    pub(crate) fn accepted(&self, text: &str) -> String {
        format!("{text}{}", self.rest(text))
    }
}

/// The built-in source of the first word: the command names.
pub(crate) fn complete_command_names(ctx: &CompleteCtx) -> Vec<String> {
    crate::COMMAND_NAMES.iter().filter(|n| n.starts_with(ctx.word)).map(|n| n.to_string()).collect()
}

/// The built-in source of `:set`'s arguments (`config::SET_ARGS`).
pub(crate) fn complete_set_args(ctx: &CompleteCtx) -> Vec<String> {
    crate::config::SET_ARGS.iter().filter(|a| a.starts_with(ctx.word)).map(|a| a.to_string()).collect()
}

/// T45/T46: the ghost for `text` with the cursor at byte offset `cursor`, or `None` (cursor not
/// at the end, empty word, a command without a `COMPLETERS` row, no match).
pub(crate) fn compute_ghost(text: &str, cursor: usize) -> Option<Ghost> {
    if cursor != text.len() {
        return None;
    }
    let word_start = crate::token_start_before(text, cursor);
    let word = &text[word_start..];
    if word.is_empty() {
        return None;
    }
    let mut earlier = text[..word_start].split_whitespace();
    let command = earlier.next();
    let arg = earlier.count() + usize::from(command.is_some());
    let ctx = CompleteCtx { line: text, word, arg };
    let (candidates, space_after) = match command {
        None => (complete_command_names(&ctx), true),
        Some(cmd) => {
            let (_, Source::Builtin(f)) = COMPLETERS.iter().find(|(name, _)| *name == cmd)?;
            (f(&ctx), SPACE_AFTER_ARG.contains(&cmd))
        }
    };
    let candidates: Vec<String> = candidates.into_iter().filter(|c| c.starts_with(word)).collect();
    if candidates.is_empty() {
        return None;
    }
    Some(Ghost { word_start, candidates, index: 0, space_after })
}
