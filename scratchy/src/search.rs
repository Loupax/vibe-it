//! QSEARCH1 (KEYS.md S1-S3, S5): find-in-page over the laid-out text.
//!
//! The finder is pure: `find_matches(lines, query)` takes `SearchLine`s (one per block of text,
//! one `SearchChar` per character with its document-space rectangle) and returns the matches in
//! document order. `collect_lines` builds the lines from a laid-out tree + the shape cache, so
//! hidden text (`display: none` has no box; `visibility: hidden` spans are masked) never matches
//! and every match already has rectangles to paint. A match may cross inline boundaries (one
//! shaped paragraph holds `foo<b>bar</b>`) but never a block boundary (one `SearchLine` per
//! block). Plain text, smartcase, no regex. Not inside iframes or text controls.

use crate::layout::{Laid, LaidContent, MixedPositioned, ShapeCache};

/// A document-space rectangle `(x, y, w, h)`.
pub(crate) type Rect = (f32, f32, f32, f32);

/// One character of a block's rendered text. `rect` is `None` for a character with no glyph of
/// its own (a collapsed or synthetic space).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SearchChar {
    pub ch: char,
    pub rect: Option<Rect>,
}

/// One block of rendered text (a shaped paragraph line group): matches never cross lines.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SearchLine {
    pub chars: Vec<SearchChar>,
}

impl SearchLine {
    /// A line from plain text with one fixed-width rect per char (tests).
    #[cfg(test)]
    pub(crate) fn from_text(text: &str, x: f32, y: f32, cw: f32, h: f32) -> SearchLine {
        SearchLine {
            chars: text
                .chars()
                .enumerate()
                .map(|(i, ch)| SearchChar { ch, rect: Some((x + i as f32 * cw, y, cw, h)) })
                .collect(),
        }
    }
}

/// The character hidden text is replaced with: it never equals a query character.
pub(crate) const HIDDEN_CHAR: char = '\u{0}';

/// One match: the line, the char range in it, and one rectangle per visual line it covers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SearchMatch {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub rects: Vec<Rect>,
}

impl SearchMatch {
    /// The top-left corner of the first rectangle (where "the match" is on the page).
    pub(crate) fn origin(&self) -> (f32, f32) {
        self.rects.first().map(|r| (r.0, r.1)).unwrap_or((0.0, 0.0))
    }
}

/// Smartcase: the search is case-insensitive unless the query has an uppercase letter.
pub(crate) fn is_case_sensitive(query: &str) -> bool {
    query.chars().any(|c| c.is_uppercase())
}

fn chars_eq(a: char, b: char, sensitive: bool) -> bool {
    a == b || (!sensitive && a.to_lowercase().eq(b.to_lowercase()))
}

/// All matches of `query` in `lines`, in document order (line order, then position); matches in
/// a line do not overlap. An empty query has none.
pub(crate) fn find_matches(lines: &[SearchLine], query: &str) -> Vec<SearchMatch> {
    let q: Vec<char> = query.chars().collect();
    if q.is_empty() {
        return Vec::new();
    }
    let sensitive = is_case_sensitive(query);
    let mut out = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let n = line.chars.len();
        let mut i = 0;
        while i + q.len() <= n {
            if q.iter().enumerate().all(|(k, &qc)| chars_eq(line.chars[i + k].ch, qc, sensitive)) {
                let end = i + q.len();
                out.push(SearchMatch { line: li, start: i, end, rects: match_rects(&line.chars[i..end]) });
                i = end;
            } else {
                i += 1;
            }
        }
    }
    out
}

/// One rectangle per visual line (same `y`) the chars cover.
fn match_rects(chars: &[SearchChar]) -> Vec<Rect> {
    let mut rects: Vec<Rect> = Vec::new();
    for r in chars.iter().filter_map(|c| c.rect) {
        match rects.last_mut() {
            Some(last) if (last.1 - r.1).abs() < 0.5 => {
                let x0 = last.0.min(r.0);
                let x1 = (last.0 + last.2).max(r.0 + r.2);
                last.0 = x0;
                last.2 = x1 - x0;
                last.3 = last.3.max(r.3);
            }
            _ => rects.push(r),
        }
    }
    rects
}

/// S5: after a relayout the current match stays the nearest one to where it was: the index of
/// the match whose origin is nearest `from` (ties: the earlier one).
pub(crate) fn nearest_match(matches: &[SearchMatch], from: (f32, f32)) -> Option<usize> {
    let d = |m: &SearchMatch| {
        let o = m.origin();
        (o.0 - from.0).powi(2) + (o.1 - from.1).powi(2)
    };
    let mut best: Option<(usize, f32)> = None;
    for (i, m) in matches.iter().enumerate() {
        let di = d(m);
        if best.is_none_or(|(_, bd)| di < bd) {
            best = Some((i, di));
        }
    }
    best.map(|(i, _)| i)
}

/// T23: the first match (document order) whose top is at or below `top` (document y), or `None`.
pub(crate) fn first_at_or_below(matches: &[SearchMatch], top: f32) -> Option<usize> {
    matches.iter().position(|m| m.origin().1 >= top)
}

/// The text of one shaped paragraph as `SearchLine`s (one per buffer line), placed at `(ox, oy)`;
/// `visibility: hidden` spans are masked.
fn buffer_lines(entry: &std::sync::Mutex<crate::layout::ShapedParagraph>, ox: f32, oy: f32) -> Vec<SearchLine> {
    let guard = entry.lock().unwrap_or_else(|e| e.into_inner());
    let (buffer, _, _, _, hidden, _, _, _) = &*guard;
    // Per buffer line: (glyph byte range, rect, hidden) from every visual line of that line.
    let mut per_line: Vec<Vec<(usize, usize, Rect, bool)>> = vec![Vec::new(); buffer.lines.len()];
    for run in buffer.layout_runs() {
        let Some(slot) = per_line.get_mut(run.line_i) else { continue };
        for g in run.glyphs {
            let h = hidden.get(g.metadata).copied().unwrap_or(false);
            slot.push((g.start, g.end, (ox + g.x, oy + run.line_top, g.w, run.line_height), h));
        }
    }
    let mut out = Vec::new();
    for (li, glyphs) in per_line.iter().enumerate() {
        let text = buffer.lines[li].text();
        let mut line = SearchLine::default();
        for (bi, ch) in text.char_indices() {
            let g = glyphs.iter().find(|g| g.0 <= bi && bi < g.1.max(g.0 + 1));
            line.chars.push(match g {
                Some(&(_, _, _, true)) => SearchChar { ch: HIDDEN_CHAR, rect: None },
                Some(&(_, _, r, false)) => SearchChar { ch, rect: Some(r) },
                None => SearchChar { ch, rect: None },
            });
        }
        out.push(line);
    }
    out
}

/// Walks the laid-out tree in document order and collects every block of text. A `position:
/// fixed` subtree is skipped (its rectangles are viewport-space, not document-space).
pub(crate) fn collect_lines(laid: &Laid, cache: &ShapeCache, out: &mut Vec<SearchLine>) {
    if laid.fixed {
        return;
    }
    match &laid.content {
        LaidContent::Blocks(children) | LaidContent::TableRow(children) => {
            for c in children {
                collect_lines(c, cache, out);
            }
        }
        LaidContent::Text(id) => {
            if let Some(entry) = cache.get(id) {
                let s = &laid.style;
                let ox = laid.x + s.border_left() + s.padding.left.px();
                let oy = laid.y + s.border_top() + s.padding.top.px();
                out.extend(buffer_lines(entry, ox, oy));
            }
        }
        LaidContent::MixedInline(items) => {
            // One inline run: its words are separate buffers, joined into one line (a space
            // between two words that do not already touch whitespace).
            let mut joined = SearchLine::default();
            let mut embedded = Vec::new();
            for item in items {
                match item {
                    MixedPositioned::Word { id, x, y, .. } => {
                        let Some(entry) = cache.get(id) else { continue };
                        for l in buffer_lines(entry, *x, *y) {
                            let prev_ws = joined.chars.last().is_none_or(|c| c.ch.is_whitespace());
                            let next_ws = l.chars.first().is_none_or(|c| c.ch.is_whitespace());
                            if !prev_ws && !next_ws {
                                joined.chars.push(SearchChar { ch: ' ', rect: None });
                            }
                            joined.chars.extend(l.chars);
                        }
                    }
                    MixedPositioned::Embedded(l) => embedded.push(l),
                }
            }
            if !joined.chars.is_empty() {
                out.push(joined);
            }
            for l in embedded {
                collect_lines(l, cache, out);
            }
        }
        LaidContent::Image(_) => {}
    }
    for p in &laid.positioned {
        collect_lines(p, cache, out);
    }
}

/// The matches of `query` in a laid-out page.
pub(crate) fn find_in_laid(laid: &Laid, cache: &ShapeCache, query: &str) -> Vec<SearchMatch> {
    let mut lines = Vec::new();
    collect_lines(laid, cache, &mut lines);
    find_matches(&lines, query)
}

/// The app's search state (S1, S3, S6): the kept query, the current matches and which one is
/// current. `highlight` is false until a search is committed and after Esc in normal mode.
#[derive(Default)]
pub(crate) struct SearchState {
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub current: Option<usize>,
    pub highlight: bool,
}

impl SearchState {
    /// T7 / T31: drop highlights and the current match; the query stays.
    pub(crate) fn clear_highlights(&mut self) {
        self.highlight = false;
        self.matches.clear();
        self.current = None;
    }

    /// S5: recompute the matches on a new layout; the current match becomes the nearest to
    /// where it was. A no-op while nothing is highlighted.
    pub(crate) fn relayout(&mut self, laid: &Laid, cache: &ShapeCache) {
        if !self.highlight || self.query.is_empty() {
            return;
        }
        let from = self.current.and_then(|i| self.matches.get(i)).map(|m| m.origin());
        self.matches = find_in_laid(laid, cache, &self.query);
        self.current = match from {
            Some(p) => nearest_match(&self.matches, p),
            None => (!self.matches.is_empty()).then_some(0),
        };
    }
}
