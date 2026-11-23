//! C-PICK: the picker (KEYS.md "Picker", T58-T68; DECISIONS.md "A picker, bookmarks, buffers as tabs,
//! popups like Chrome").
//!
//! A list the shell draws over the page, just above the status bar (the page is not resized), opened
//! only by dedicated commands (`:historySearch` here; `:bookmarkSearch`, `:bufferList`, `:popupSearch`
//! later). Its own mode, PICK (`VimMode::Pick`): the bar shows the filter being typed, the page gets no
//! keys, Ctrl-n/Down and Ctrl-p/Up move the selection, Enter runs the selected row's action and closes,
//! Esc closes.
//!
//! **Adding a source** (C-BOOKMARKS, C4, C-POPUPS): write a function that returns `Vec<Row>` in the
//! source's own order (that order breaks ties between equally good matches), give each row the texts
//! the filter is matched against (`fields`), and the action Enter runs (a `PickAction`; add a variant
//! and its arm in `Keys::run_pick_action` if none fits). Then open it from a command with
//! `Keys::open_picker(name, rows, filter, empty_message)`. Matching, ranking, keys, drawing and the
//! scale are shared by every source.
//!
//! Pure parts (matching, ranking, selection, geometry) are tested in `tests/c_pick_tests.rs`; the
//! drawing reuses the status bar's `Painter` (cosmic-text + tiny-skia) at the window's scale.

use std::path::Path;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size, Transform};
use tiny_skia::{Pixmap, PixmapPaint};

use crate::config::{
    GHOST_FG, PICKER_BG, PICKER_HEADER_FG, PICKER_MAX_ROWS, PICKER_ROW_H, PICKER_SEL_BG, STATUS_BAR_FG, STATUS_BAR_GAP, STATUS_BAR_MODE_FG,
    STATUS_BAR_PAD_X,
};
use crate::keys::{Effect, KeyInput, Keys, VimMode};
use crate::layout::Layout;
use crate::protocol::Command;
use crate::statusbar::{elide_end, elide_middle, Painter};

// ---------- rows and sources ----------

/// What Enter does with a row (T62). Later sources add variants (a buffer to show, a blocked popup
/// to open) and their arm in `Keys::run_pick_action`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickAction {
    /// Load this URL in the current page (history, bookmarks).
    Open(String),
    /// Show this buffer (C4, `:bufferList`).
    Buffer(u32),
}

/// One row of a source: what is drawn (the label, then the dimmed detail), what the filter is
/// matched against (`fields`, the source's choice) and what Enter runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub detail: String,
    pub fields: Vec<String>,
    pub action: PickAction,
}

/// `:historySearch`'s rows from the history file's text (STASH.md "History"): the `:go` ranking
/// (`completion::rank_history`: site roots by visits to the site, then pages by their own visits,
/// ties newest first). Label: the newest non-empty title recorded for exactly that URL (a site root
/// without one: its host); a page with no title shows its URL as the label. Detail: the URL.
/// Matched on the URL without `https://`/`http://`/`www.`, and the title.
pub fn history_rows(text: &str) -> Vec<Row> {
    let mut titles: std::collections::HashMap<&str, (u64, &str)> = std::collections::HashMap::new();
    for line in text.lines() {
        let mut f = line.splitn(3, '\t');
        let (Some(ms), Some(url), title) = (f.next(), f.next(), f.next().unwrap_or("").trim()) else {
            continue;
        };
        let Ok(ms) = ms.trim().parse::<u64>() else { continue };
        if title.is_empty() {
            continue;
        }
        let e = titles.entry(url).or_insert((0, ""));
        if ms >= e.0 {
            *e = (ms, title);
        }
    }
    crate::completion::rank_history(text)
        .into_iter()
        .map(|e| {
            let title = match titles.get(e.url.as_str()) {
                Some((_, t)) => t.to_string(),
                // A site root with no title of its own: its host (STASH.md "Ranking").
                None if url::Url::parse(&e.url).is_ok_and(|u| u.path() == "/" && u.query().is_none()) => url::Url::parse(&e.url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_string))
                    .unwrap_or_default(),
                None => String::new(),
            };
            let bare = e.url[e.skip..].to_string();
            let (label, detail) = if title.is_empty() {
                (e.url.clone(), String::new())
            } else {
                (title.clone(), e.url.clone())
            };
            Row {
                label,
                detail,
                fields: vec![bare, title],
                action: PickAction::Open(e.url),
            }
        })
        .collect()
}

/// `history_rows` of `DIR/history` (nothing without a state directory or file).
pub fn history_source(dir: Option<&Path>) -> Vec<Row> {
    let data = dir.and_then(|d| std::fs::read(d.join("history")).ok()).unwrap_or_default();
    history_rows(&String::from_utf8_lossy(&data))
}

// ---------- fuzzy matching (T59) ----------

/// Score of a matched character.
const SCORE_MATCH: i32 = 16;
/// Bonus for a match at the start of the text or of a word (after a separator).
const BONUS_BOUNDARY: i32 = 8;
/// Bonus for a capital after a lower-case letter (camelCase).
const BONUS_CAMEL: i32 = 7;
/// Least bonus for a match right after the previous one (a run otherwise keeps the bonus of its
/// first character).
const BONUS_CONSECUTIVE: i32 = 4;
/// Penalty for the first skipped character between two matches, and for each one after it.
const GAP_START: i32 = 3;
const GAP_EXTENSION: i32 = 1;
/// Texts are matched on their first this many characters (a long URL's tail never decides).
const MAX_TEXT: usize = 512;

fn is_separator(c: char) -> bool {
    !c.is_alphanumeric()
}

/// The bonus for a match at `i` (`text` as chars): word starts and camel humps score higher.
fn position_bonus(text: &[char], i: usize) -> i32 {
    let Some(prev) = i.checked_sub(1).map(|p| text[p]) else {
        return BONUS_BOUNDARY;
    };
    let c = text[i];
    if is_separator(prev) && !is_separator(c) {
        BONUS_BOUNDARY
    } else if prev.is_lowercase() && c.is_uppercase() {
        BONUS_CAMEL
    } else {
        0
    }
}

/// fzf-style fuzzy score of `pattern` in `text`: the pattern's characters must appear in `text` in
/// order (not necessarily together); `None` when they do not. Higher is better: each matched
/// character scores, matches at word starts and right after the previous match score more, gaps
/// between matches cost. Smartcase as in Search: case matters only when the pattern has a capital.
/// The best placement of the pattern is found (dynamic programming over the text).
pub fn fuzzy_score(pattern: &str, text: &str) -> Option<i32> {
    let case = pattern.chars().any(char::is_uppercase);
    let fold = |c: char| if case { c } else { c.to_lowercase().next().unwrap_or(c) };
    let p: Vec<char> = pattern.chars().map(fold).collect();
    if p.is_empty() {
        return Some(0);
    }
    let orig: Vec<char> = text.chars().take(MAX_TEXT).collect();
    let t: Vec<char> = orig.iter().map(|&c| fold(c)).collect();
    // Quick rejection: a greedy left-to-right scan finds a placement iff one exists.
    let mut k = 0;
    for &c in &t {
        if k < p.len() && c == p[k] {
            k += 1;
        }
    }
    if k < p.len() {
        return None;
    }
    let n = t.len();
    const NONE: i32 = i32::MIN / 2;
    // prev[j]: best score with p[..i] placed and p[i-1] at t[j]; prev_chunk[j]: the bonus of the
    // first character of the run of consecutive matches ending there (fzf: a run keeps the bonus of
    // its start, so "docs" after a `/` beats d-o-c spread over three word starts).
    let mut prev = vec![NONE; n];
    let mut prev_chunk = vec![0; n];
    for (i, &pc) in p.iter().enumerate() {
        let mut cur = vec![NONE; n];
        let mut cur_chunk = vec![0; n];
        // Best `prev[k] - gap(j - 1 - k)` over k < j - 1 (at least one character skipped).
        let mut gapped = NONE;
        for j in 0..n {
            if j >= 2 && prev[j - 2] > NONE {
                gapped = gapped.max(prev[j - 2] - GAP_START);
            }
            if t[j] == pc {
                let bonus = position_bonus(&orig, j);
                if i == 0 {
                    cur[j] = SCORE_MATCH + 2 * bonus;
                    cur_chunk[j] = bonus;
                } else {
                    let from_gap = if gapped > NONE { gapped + SCORE_MATCH + bonus } else { NONE };
                    let (from_run, run_bonus) = if j >= 1 && prev[j - 1] > NONE {
                        let b = bonus.max(prev_chunk[j - 1]).max(BONUS_CONSECUTIVE);
                        (prev[j - 1] + SCORE_MATCH + b, if bonus >= BONUS_BOUNDARY { bonus } else { b })
                    } else {
                        (NONE, 0)
                    };
                    if from_run >= from_gap && from_run > NONE {
                        cur[j] = from_run;
                        cur_chunk[j] = run_bonus;
                    } else if from_gap > NONE {
                        cur[j] = from_gap;
                        cur_chunk[j] = bonus;
                    }
                }
            }
            if gapped > NONE {
                gapped -= GAP_EXTENSION;
            }
        }
        prev = cur;
        prev_chunk = cur_chunk;
    }
    prev.into_iter().filter(|&s| s > NONE).max()
}

/// A row's score for a filter: every whitespace-separated term must match one of the row's fields
/// fuzzily (each term in its best field); the score is the sum. An empty filter matches with 0.
pub fn row_score(filter: &str, fields: &[String]) -> Option<i32> {
    let mut total = 0;
    for term in filter.split_whitespace() {
        total += fields.iter().filter_map(|f| fuzzy_score(term, f)).max()?;
    }
    Some(total)
}

/// T59: the indices of the rows that match `filter`, best score first, ties in the source's order.
pub fn rank(rows: &[Row], filter: &str) -> Vec<usize> {
    let mut scored: Vec<(i32, usize)> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| row_score(filter, &r.fields).map(|s| (s, i)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

// ---------- the picker's state ----------

/// One open picker: the source's rows, the matches for the filter and the selection.
#[derive(Debug, Clone)]
pub struct Picker {
    /// The source's name, shown in the list's header (`history`).
    pub name: String,
    rows: Vec<Row>,
    /// Indices into `rows`, best first.
    matches: Vec<usize>,
    /// Position in `matches` (meaningless when there are none).
    pub selected: usize,
    /// The first match shown (the list scrolls to keep the selection visible).
    top: usize,
}

impl Picker {
    pub fn new(name: &str, rows: Vec<Row>) -> Picker {
        let matches = (0..rows.len()).collect();
        Picker {
            name: name.to_string(),
            rows,
            matches,
            selected: 0,
            top: 0,
        }
    }

    /// T59: the filter changed: re-rank, select the best match.
    pub fn set_filter(&mut self, filter: &str) {
        self.matches = rank(&self.rows, filter);
        self.selected = 0;
        self.top = 0;
    }

    /// How many rows the source has.
    pub fn total(&self) -> usize {
        self.rows.len()
    }

    /// The matching rows, best first.
    #[cfg(test)]
    pub fn matches(&self) -> impl Iterator<Item = &Row> {
        self.matches.iter().map(|&i| &self.rows[i])
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// The row Enter would run, if any matches.
    pub fn selected_row(&self) -> Option<&Row> {
        self.matches.get(self.selected).map(|&i| &self.rows[i])
    }

    /// T60/T61: moves the selection by `by` rows, stopping at the first and last match (no wrap).
    /// The list scrolls so the selection stays among the `PICKER_MAX_ROWS` shown.
    pub fn move_by(&mut self, by: isize) {
        let n = self.matches.len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + by).clamp(0, n as isize - 1) as usize;
        self.top = scroll_top(self.top, self.selected, PICKER_MAX_ROWS);
    }

    /// What is drawn with room for `room` rows: the header and the rows shown (the selection kept
    /// visible).
    pub fn view(&self, room: usize) -> PickView {
        let shown = room.min(self.matches.len());
        let top = scroll_top(self.top, self.selected, shown.max(1)).min(self.matches.len().saturating_sub(shown));
        let rows = self.matches[top..top + shown]
            .iter()
            .map(|&i| (self.rows[i].label.clone(), self.rows[i].detail.clone()))
            .collect();
        PickView {
            name: self.name.clone(),
            count: format!("{}/{}", self.matches.len(), self.rows.len()),
            rows,
            selected: (shown > 0).then(|| self.selected - top),
        }
    }
}

/// The first row shown so that `selected` is among `shown` rows starting there, moving `top` as
/// little as possible.
pub fn scroll_top(top: usize, selected: usize, shown: usize) -> usize {
    let shown = shown.max(1);
    if selected < top {
        selected
    } else if selected >= top + shown {
        selected + 1 - shown
    } else {
        top
    }
}

/// What the overlay shows (compared to skip redrawing an unchanged list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickView {
    pub name: String,
    /// `matches/total`.
    pub count: String,
    /// (label, detail) of the rows shown, top to bottom.
    pub rows: Vec<(String, String)>,
    /// Which of `rows` is selected.
    pub selected: Option<usize>,
}

// ---------- geometry (logical px; the physical rectangle at the window's scale) ----------

/// How many rows fit over the page: at most `PICKER_MAX_ROWS`, and the header plus the rows never
/// taller than the page.
pub fn room(layout: &Layout) -> usize {
    let fit = (layout.page().size.h / PICKER_ROW_H - 1).max(0) as usize;
    PICKER_MAX_ROWS.min(fit)
}

/// The overlay's rectangle for `rows` rows shown: the page's width, the header plus the rows tall,
/// its bottom on the page's bottom edge (right above the status bar). None when nothing fits.
pub fn overlay_rect(layout: &Layout, rows: usize) -> Option<Rectangle<i32, Logical>> {
    let page = layout.page();
    let h = (rows as i32 + 1) * PICKER_ROW_H;
    if h > page.size.h {
        return None;
    }
    Some(Rectangle::new(
        (page.loc.x, page.loc.y + page.size.h - h).into(),
        (page.size.w, h).into(),
    ))
}

/// Where the overlay's pixels go in the framebuffer at `scale`: each edge rounded on its own, as
/// the status bar's (`Layout::bar_physical`), so it meets the bar with no gap or overlap.
pub fn to_physical(rect: Rectangle<i32, Logical>, scale: f64) -> (Point<i32, Physical>, Size<i32, Physical>) {
    let r = |v: i32| (v as f64 * scale).round() as i32;
    let (x0, y0) = (r(rect.loc.x), r(rect.loc.y));
    let (x1, y1) = (r(rect.loc.x + rect.size.w), r(rect.loc.y + rect.size.h));
    ((x0, y0).into(), ((x1 - x0).max(1), (y1 - y0).max(1)).into())
}

/// The overlay of the open picker in this layout (None: no picker, or no room).
pub fn current_rect(keys: &Keys, layout: &Layout) -> Option<Rectangle<i32, Logical>> {
    let p = keys.picker.as_ref()?;
    overlay_rect(layout, room(layout).min(p.match_count()))
}

// ---------- keys (T58-T68) ----------

impl Keys {
    /// T58: opens a picker over `rows` with `filter` typed. No rows: stays where it was and the
    /// status bar says `empty`.
    pub fn open_picker(&mut self, name: &str, rows: Vec<Row>, filter: &str, empty: &str) {
        if rows.is_empty() {
            self.message = empty.to_string();
            return;
        }
        let mut p = Picker::new(name, rows);
        self.line.clear();
        self.line.insert_str(filter);
        p.set_filter(&self.line.text);
        self.ghost = None;
        self.picker = Some(p);
        self.vim_mode = VimMode::Pick;
    }

    /// `:historySearch [filter]` (C-PICK).
    pub fn history_search(&mut self, filter: &str) {
        let rows = history_source(self.state_dir().as_deref());
        self.open_picker("history", rows, filter, "web: the history is empty");
    }

    /// `:bookmarkAdd` (T69): the current page, with the host's title (none when the host gave only
    /// the URL, as the history does); a URL already there gets its title updated.
    pub fn bookmark_add(&mut self) {
        let url = self.url.clone();
        let Some(dir) = self.state_dir() else {
            self.message = "web: no state directory: bookmarks are off".to_string();
            return;
        };
        if url.is_empty() || url == "about:blank" {
            self.message = "web: no page to bookmark".to_string();
            return;
        }
        let bare = |u: &str| u.split_once("://").map_or(u, |(_, r)| r).to_string();
        let title = if self.title == url || self.title == bare(&url) {
            ""
        } else {
            self.title.as_str()
        };
        self.message = match crate::bookmarks::add(&dir, &url, title) {
            Ok(crate::bookmarks::Added::New) => format!("bookmarked: {url}"),
            Ok(crate::bookmarks::Added::Updated) => format!("bookmark updated: {url}"),
            Err(e) => format!("web: bookmark failed: {e}"),
        };
    }

    /// `:bookmarkDelete [url]` (T70, T71).
    pub fn bookmark_delete(&mut self, url: &str) {
        let Some(dir) = self.state_dir() else {
            self.message = "web: no state directory: bookmarks are off".to_string();
            return;
        };
        if url.is_empty() {
            self.message = "web: no page: give the bookmark's URL".to_string();
            return;
        }
        self.message = match crate::bookmarks::delete(&dir, url) {
            Ok(true) => format!("bookmark removed: {url}"),
            Ok(false) => format!("web: not bookmarked: {url}"),
            Err(e) => format!("web: bookmark failed: {e}"),
        };
    }

    /// `:bookmarkSearch [filter]` (T72).
    pub fn bookmark_search(&mut self, filter: &str) {
        let rows = crate::bookmarks::rows(&crate::bookmarks::load(self.state_dir().as_deref()));
        self.open_picker("bookmarks", rows, filter, "web: no bookmarks (:bookmarkAdd adds the page)");
    }

    /// A key in PICK (T59-T65).
    pub(crate) fn pick_key(&mut self, key: &KeyInput) {
        use smithay::input::keyboard::Keysym as K;
        let s = key.sym;
        let plain = !key.ctrl && !key.alt && !key.logo;
        if s == K::Escape {
            // T64.
            self.close_line();
        } else if s == K::Return || s == K::KP_Enter {
            // T62/T63.
            if let Some(action) = self.picker.as_ref().and_then(|p| p.selected_row()).map(|r| r.action.clone()) {
                self.close_line();
                self.run_pick_action(action);
            }
        } else if (plain && (s == K::Down || s == K::KP_Down)) || (key.ctrl && key.ch == Some('n')) {
            // T60.
            if let Some(p) = &mut self.picker {
                p.move_by(1);
            }
        } else if (plain && (s == K::Up || s == K::KP_Up)) || (key.ctrl && key.ch == Some('p')) {
            // T61.
            if let Some(p) = &mut self.picker {
                p.move_by(-1);
            }
        } else if s == K::Right || s == K::End || s == K::KP_Right || s == K::KP_End {
            if s == K::End || s == K::KP_End {
                self.line.cursor = self.line.text.len();
            } else {
                self.line.move_right();
            }
        } else if s != K::Tab && s != K::ISO_Left_Tab && s != K::Delete && s != K::KP_Delete {
            // T59 (T65: Tab, Shift-Tab and Delete do nothing).
            let before = self.line.text.clone();
            self.line_edit_key(key);
            if self.line.text != before {
                let filter = self.line.text.clone();
                if let Some(p) = &mut self.picker {
                    p.set_filter(&filter);
                }
            }
        }
    }

    /// T62: what Enter on a row does.
    fn run_pick_action(&mut self, action: PickAction) {
        match action {
            PickAction::Open(url) => self.effects.push(Effect::Send(Command::Open { url })),
            PickAction::Buffer(n) => {
                if self.buffers.get(n).is_some() {
                    self.switch_to(n)
                }
            }
        }
    }

    /// T66/T67: a mouse press while the picker is open closes it. Returns whether the press (and
    /// its release) must be kept from the page: it was over the list.
    pub fn pick_click(&mut self, over_list: bool) -> bool {
        if self.vim_mode != VimMode::Pick {
            return false;
        }
        self.close_line();
        over_list
    }

    /// The picker's state for the shell's log (the live check reads it): empty when closed.
    pub fn pick_log(&self) -> String {
        let Some(p) = &self.picker else { return String::new() };
        let sel = p
            .selected_row()
            .map(|r| match &r.action {
                PickAction::Open(u) => u.clone(),
                PickAction::Buffer(n) => format!("buffer {n}"),
            })
            .unwrap_or_default();
        let at = if p.match_count() == 0 { 0 } else { p.selected + 1 };
        format!(" pick {} [{at}/{} of {}] {sel}", p.name, p.match_count(), p.total())
    }
}

// ---------- drawing ----------

fn color(c: (u8, u8, u8)) -> cosmic_text::Color {
    cosmic_text::Color::rgb(c.0, c.1, c.2)
}

/// The overlay's pixels, `w` x `h` physical px at `scale` (sizes in config.rs are logical): a header
/// row (`PICK`, the source's name, `matches/total`) and then the rows, the selected one on a lighter
/// background; each row is its label, then its detail dimmed, both elided to fit.
pub fn paint(painter: &mut Painter, view: &PickView, w: u32, h: u32, scale: f32) -> Option<Pixmap> {
    painter.scale = scale;
    let mut pm = Pixmap::new(w.max(1), h.max(1))?;
    pm.fill(tiny_skia::Color::from_rgba8(PICKER_BG.0, PICKER_BG.1, PICKER_BG.2, 255));
    let n = view.rows.len() as u32 + 1;
    // Row i covers [edge(i), edge(i + 1)): the rows tile the overlay exactly at any scale.
    let edge = |i: u32| ((i as f32 * h as f32 / n as f32).round()) as u32;
    let (pad, gap) = (STATUS_BAR_PAD_X * scale, STATUS_BAR_GAP * scale);
    let avail = (w as f32 - 2.0 * pad).max(0.0);
    for i in 0..n {
        let rh = edge(i + 1).saturating_sub(edge(i)).max(1);
        let mut row = Pixmap::new(w.max(1), rh)?;
        let selected = i > 0 && view.selected == Some(i as usize - 1);
        let bg = if selected { PICKER_SEL_BG } else { PICKER_BG };
        row.fill(tiny_skia::Color::from_rgba8(bg.0, bg.1, bg.2, 255));
        if i == 0 {
            let x = pad + painter.text(&mut row, "PICK", pad, avail, color(STATUS_BAR_MODE_FG)) + gap;
            let nx = x + painter.text(&mut row, &view.name, x, (pad + avail - x).max(0.0), color(PICKER_HEADER_FG)) + gap;
            painter.text(&mut row, &view.count, nx, (pad + avail - nx).max(0.0), color(GHOST_FG));
        } else {
            let (label, detail) = &view.rows[i as usize - 1];
            let label_max = if detail.is_empty() { avail } else { avail * 0.6 };
            let label = elide_end(label, label_max, &mut |t| painter.measure(t));
            let lw = painter.text(&mut row, &label, pad, label_max, color(STATUS_BAR_FG));
            if !detail.is_empty() {
                let x = pad + lw + gap;
                let room = (pad + avail - x).max(0.0);
                let detail = elide_middle(detail, room, &mut |t| painter.measure(t));
                painter.text(&mut row, &detail, x, room + 0.5, color(GHOST_FG));
            }
        }
        pm.draw_pixmap(
            0,
            edge(i) as i32,
            row.as_ref(),
            &PixmapPaint::default(),
            tiny_skia::Transform::identity(),
            None,
        );
    }
    Some(pm)
}

/// The overlay as the compositor draws it: a memory buffer redrawn when the view, size or scale
/// changes (as `statusbar::Bar`).
pub struct Overlay {
    buffer: MemoryRenderBuffer,
    drawn: Option<(PickView, u32, u32, u32)>,
}

impl Overlay {
    pub fn new() -> Overlay {
        Overlay {
            buffer: MemoryRenderBuffer::new(Fourcc::Abgr8888, (1, 1), 1, Transform::Normal, None),
            drawn: None,
        }
    }

    /// The render element for the open picker, painted at the framebuffer's resolution and drawn
    /// 1:1 into its logical rectangle (as the bar, C6b). None when no picker is open.
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        keys: &Keys,
        layout: &Layout,
        bar: &mut crate::statusbar::Bar,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let picker = keys.picker.as_ref()?;
        let room = room(layout);
        let view = picker.view(room);
        let rect = overlay_rect(layout, view.rows.len())?;
        let (loc, px) = to_physical(rect, layout.scale);
        let (w, h) = (px.w as u32, px.h as u32);
        let s120 = (layout.scale * 120.0).round() as u32;
        if !self
            .drawn
            .as_ref()
            .is_some_and(|(v, dw, dh, ds)| v == &view && *dw == w && *dh == h && *ds == s120)
        {
            let pm = paint(bar.painter(), &view, w, h, layout.scale as f32)?;
            let (pw, ph) = (pm.width() as i32, pm.height() as i32);
            let mut ctx = self.buffer.render();
            ctx.resize((pw, ph));
            let _ = ctx.draw(|mem| {
                mem.copy_from_slice(pm.data());
                Ok::<_, ()>(vec![Rectangle::from_size((pw, ph).into())])
            });
            ctx.update_opaque_regions(Some(vec![Rectangle::from_size((pw, ph).into())]));
            drop(ctx);
            self.drawn = Some((view, w, h, s120));
        }
        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            loc.to_f64(),
            &self.buffer,
            None,
            Some(Rectangle::from_size((px.w as f64, px.h as f64).into())),
            Some(rect.size),
            Kind::Unspecified,
        ) {
            Ok(e) => Some(e),
            Err(e) => {
                eprintln!("[web] picker: {e:?}");
                None
            }
        }
    }
}
