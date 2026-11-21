//! Q30g: live `<select>` state, the dropdown popup and listbox picking.
//!
//! DESIGN NOTE (the user will redesign the look and behaviour later; every
//! piece is a small function, every colour/size is a constant in the
//! "look" block below): same pattern as Q30c/Q30d. The layout tree only SIZES
//! a select (`forms::build_select`); the selected text, the arrow, the listbox
//! rows and the popup are painted from the live selection, so opening,
//! moving and choosing never rebuild the layout.
//!
//! * Selection state: one thread-local map keyed by the select's `node_id`
//!   holding the selected options' `node_id`s once the user/a script touched
//!   it ("dirty"); until then the `selected` attributes decide (a single
//!   select shows the LAST `selected` option, else the first non-disabled one;
//!   a `multiple`/listbox select just the `selected` ones). `selected_options`
//!   in forms_submit.rs and the JS natives read it through
//!   `selected_indices`.
//! * The painter has no DOM access, so `build_select` registers a snapshot
//!   (`SelSnap`: labels, disabled, attribute selection) per select.
//! * The popup (a single `<select>`) is `App::select_popup`: a pure
//!   `PopupState` plus the geometry function `popup_geometry`; it is painted
//!   LAST (above everything, like the command bar) and hit-tested before the
//!   page, so it is never part of the layout flow.
//! * Keyboard: a focused select owns the keys like a text control does (Tab
//!   passes, Esc blurs); an open popup owns every key, Esc closes it first.

use super::*;

pub(crate) const SEL_DROP: u8 = 1;
pub(crate) const SEL_LIST: u8 = 2;

// ---- the look (one place; the user will redesign it) ------------------------
/// Width reserved for the drop arrow at the right of a closed select.
pub(crate) const SELECT_ARROW_W: f32 = 18.0;
/// At most this many rows are visible in the popup before it scrolls.
pub(crate) const POPUP_MAX_ROWS: usize = 10;
const POPUP_ROW_PAD: f32 = 4.0;
const POPUP_TEXT_PAD: f32 = 6.0;
const POPUP_GROUP_INDENT: f32 = 12.0;
const POPUP_SCROLLBAR_W: f32 = 8.0;
const POPUP_BORDER: (u8, u8, u8) = (118, 118, 118);
const POPUP_BG: (u8, u8, u8) = (255, 255, 255);
const POPUP_SELECTED_BG: (u8, u8, u8) = (232, 232, 232);
const POPUP_HILITE_TEXT: (u8, u8, u8) = (255, 255, 255);
const POPUP_DISABLED_TEXT: (u8, u8, u8) = (117, 117, 117);
const POPUP_HEADER_TEXT: (u8, u8, u8) = (70, 70, 70);
const LIST_SELECTED_BLURRED_BG: (u8, u8, u8) = (206, 206, 206);

// ---- the option list ------------------------------------------------------

/// One row of the option list: an `<option>` (`opt` = its index among the
/// options) or an `<optgroup>` label (`opt` None, never selectable).
#[derive(Clone, Debug)]
pub(crate) struct SelRow {
    pub(crate) opt: Option<usize>,
    pub(crate) label: String,
    pub(crate) disabled: bool,
    pub(crate) grouped: bool,
}

/// What `resolve_selection` needs of one option.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OptSnap {
    pub(crate) node: usize,
    pub(crate) disabled: bool,
    pub(crate) attr_selected: bool,
}

/// Everything the painter (no DOM) needs about a select.
#[derive(Clone, Debug)]
pub(crate) struct SelSnap {
    pub(crate) rows: Vec<SelRow>,
    pub(crate) opts: Vec<OptSnap>,
    pub(crate) multiple: bool,
    pub(crate) listbox: bool,
}

impl SelSnap {
    pub(crate) fn widest_label_chars(&self) -> usize {
        self.rows.iter().map(|r| r.label.chars().count()).max().unwrap_or(0)
    }
}

fn option_label(o: &Handle) -> String {
    match attr_string(o, "label").filter(|l| !l.is_empty()) {
        Some(l) => l,
        None => {
            let mut t = String::new();
            collect_text_only(o, &mut t);
            t.split_whitespace().collect::<Vec<_>>().join(" ")
        }
    }
}

pub(crate) fn is_multiple(select: &Handle) -> bool {
    element_attr(select, "multiple").is_some()
}

/// `multiple` or `size > 1`: drawn as a list in the page, no popup.
pub(crate) fn is_listbox(select: &Handle) -> bool {
    is_multiple(select) || attr_string(select, "size").and_then(|s| s.trim().parse::<u32>().ok()).is_some_and(|s| s > 1)
}

/// The rows, the option snapshots and the option handles of `select` in tree
/// order (options directly inside it or inside an `<optgroup>`).
pub(crate) fn select_data(select: &Handle) -> (SelSnap, Vec<Handle>) {
    fn walk(h: &Handle, group_disabled: bool, grouped: bool, snap: &mut SelSnap, handles: &mut Vec<Handle>) {
        for c in h.children.borrow().iter() {
            if !matches!(c.data, NodeData::Element { .. }) {
                continue;
            }
            match element_tag(c) {
                "option" => {
                    let disabled = group_disabled || element_attr(c, "disabled").is_some();
                    snap.rows.push(SelRow { opt: Some(handles.len()), label: option_label(c), disabled, grouped });
                    snap.opts.push(OptSnap { node: node_id(c), disabled, attr_selected: element_attr(c, "selected").is_some() });
                    handles.push(c.clone());
                }
                "optgroup" if !grouped => {
                    let dis = group_disabled || element_attr(c, "disabled").is_some();
                    let label = attr_string(c, "label").unwrap_or_default();
                    snap.rows.push(SelRow { opt: None, label, disabled: true, grouped: false });
                    walk(c, dis, true, snap, handles);
                }
                _ => {}
            }
        }
    }
    let mut snap = SelSnap { rows: Vec::new(), opts: Vec::new(), multiple: is_multiple(select), listbox: is_listbox(select) };
    let mut handles = Vec::new();
    walk(select, false, false, &mut snap, &mut handles);
    (snap, handles)
}

pub(crate) fn select_options(select: &Handle) -> Vec<Handle> {
    select_data(select).1
}

pub(crate) fn select_rows(select: &Handle) -> Vec<SelRow> {
    select_data(select).0.rows
}

// ---- live selection -------------------------------------------------------

#[derive(Default, Clone)]
struct SelLive {
    /// Selected options' node ids; `None` = not dirty (the attributes rule).
    sel: Option<Vec<usize>>,
    /// Listbox: first visible row.
    scroll: usize,
    /// Listbox: range anchor and keyboard cursor (option indices).
    anchor: Option<usize>,
    cursor: Option<usize>,
}

thread_local! {
    static SEL_LIVE: RefCell<HashMap<usize, SelLive>> = RefCell::new(HashMap::new());
    static SEL_SNAP: RefCell<HashMap<usize, Rc<SelSnap>>> = RefCell::new(HashMap::new());
}

pub(crate) fn clear_select_state() {
    SEL_LIVE.with(|m| m.borrow_mut().clear());
    SEL_SNAP.with(|m| m.borrow_mut().clear());
}

/// Called whenever the box of a select is (re)built.
pub(crate) fn register_select(select: &Handle) -> Rc<SelSnap> {
    let snap = Rc::new(select_data(select).0);
    SEL_SNAP.with(|m| m.borrow_mut().insert(node_id(select), snap.clone()));
    snap
}

/// The selected option indices, pure: `live` = the dirty selection (node ids),
/// `None` = attributes. Single selects get at most one; "ask for a reset" gives a
/// non-listbox single select its first non-disabled option when nothing is selected.
pub(crate) fn resolve_selection(opts: &[OptSnap], multiple: bool, listbox: bool, live: Option<&[usize]>) -> Vec<usize> {
    let default = || -> Vec<usize> {
        if multiple {
            return opts.iter().enumerate().filter(|(_, o)| o.attr_selected).map(|(i, _)| i).collect();
        }
        if let Some(i) = opts.iter().rposition(|o| o.attr_selected) {
            return vec![i];
        }
        if !listbox {
            return opts.iter().position(|o| !o.disabled).into_iter().collect();
        }
        Vec::new()
    };
    match live {
        None => default(),
        Some(ids) => {
            let idx: Vec<usize> = opts.iter().enumerate().filter(|(_, o)| ids.contains(&o.node)).map(|(i, _)| i).collect();
            if idx.is_empty() && !ids.is_empty() {
                // Every selected option was removed: back to the defaults.
                default()
            } else if !multiple && idx.len() > 1 {
                vec![*idx.last().unwrap()]
            } else {
                idx
            }
        }
    }
}

fn live_ids(select_node: usize) -> Option<Vec<usize>> {
    SEL_LIVE.with(|m| m.borrow().get(&select_node).and_then(|l| l.sel.clone()))
}

/// The selected option indices of `select` (live, else the attributes).
pub(crate) fn selected_indices(select: &Handle) -> Vec<usize> {
    let (snap, _) = select_data(select);
    resolve_selection(&snap.opts, snap.multiple, snap.listbox, live_ids(node_id(select)).as_deref())
}

pub(crate) fn selected_option_handles(select: &Handle) -> Vec<Handle> {
    let (snap, handles) = select_data(select);
    resolve_selection(&snap.opts, snap.multiple, snap.listbox, live_ids(node_id(select)).as_deref())
        .into_iter()
        .map(|i| handles[i].clone())
        .collect()
}

/// `select.selectedIndex`: the first selected option, -1 when none.
pub(crate) fn selected_index(select: &Handle) -> i32 {
    selected_indices(select).first().map_or(-1, |i| *i as i32)
}

/// Replaces the live selection (making it dirty). Returns whether it changed.
pub(crate) fn set_selected_indices(select: &Handle, idx: &[usize]) -> bool {
    let (snap, handles) = select_data(select);
    let before = resolve_selection(&snap.opts, snap.multiple, snap.listbox, live_ids(node_id(select)).as_deref());
    let mut idx: Vec<usize> = idx.iter().copied().filter(|i| *i < handles.len()).collect();
    idx.sort_unstable();
    idx.dedup();
    if !snap.multiple && idx.len() > 1 {
        idx = vec![*idx.last().unwrap()];
    }
    let ids: Vec<usize> = idx.iter().map(|i| node_id(&handles[*i])).collect();
    SEL_LIVE.with(|m| m.borrow_mut().entry(node_id(select)).or_default().sel = Some(ids));
    before != idx
}

/// The `<select>` owning `<option>` `o` (directly or through an `<optgroup>`).
pub(crate) fn option_owner(o: &Handle) -> Option<Handle> {
    let mut cur = parent_handle(o);
    while let Some(n) = cur {
        match element_tag(&n) {
            "select" => return Some(n),
            "optgroup" => cur = parent_handle(&n),
            _ => return None,
        }
    }
    None
}

/// `option.selected` (the attribute for an option outside a select).
pub(crate) fn option_is_selected(o: &Handle) -> bool {
    let Some(sel) = option_owner(o) else { return element_attr(o, "selected").is_some() };
    let handles = select_options(&sel);
    let Some(i) = handles.iter().position(|h| Rc::ptr_eq(h, o)) else { return false };
    selected_indices(&sel).contains(&i)
}

/// `option.selected = v`. Returns whether the selection changed.
pub(crate) fn set_option_selected(o: &Handle, v: bool) -> bool {
    let Some(sel) = option_owner(o) else { return false };
    let (snap, handles) = select_data(&sel);
    let Some(i) = handles.iter().position(|h| Rc::ptr_eq(h, o)) else { return false };
    let mut cur = selected_indices(&sel);
    if snap.multiple {
        if v {
            cur.push(i);
        } else {
            cur.retain(|x| *x != i);
        }
    } else if v {
        cur = vec![i];
    } else if cur == [i] {
        // Deselecting the only selection of a single select: the reset rule.
        cur = if snap.listbox { Vec::new() } else { snap.opts.iter().position(|o| !o.disabled).into_iter().collect() };
    }
    set_selected_indices(&sel, &cur)
}

fn all_selects(h: &Handle, out: &mut Vec<Handle>) {
    if element_tag(h) == "select" {
        out.push(h.clone());
    }
    for c in h.children.borrow().iter() {
        all_selects(c, out);
    }
}

/// `form.reset()`: the selects owned by `form` go back to their `selected`
/// attributes. True if any had live state.
pub(crate) fn reset_select_state(form: &Handle) -> bool {
    let mut top = form.clone();
    while let Some(p) = parent_handle(&top) {
        top = p;
    }
    let mut all = Vec::new();
    all_selects(&top, &mut all);
    let mut any = false;
    for s in all {
        if form_owner(&s).is_some_and(|f| Rc::ptr_eq(&f, form)) {
            any |= SEL_LIVE.with(|m| m.borrow_mut().remove(&node_id(&s))).is_some_and(|l| l.sel.is_some());
        }
    }
    any
}

// ---- what the painter reads -----------------------------------------------

pub(crate) struct SelPaint {
    pub(crate) snap: Rc<SelSnap>,
    /// Selected option indices.
    pub(crate) selected: Vec<usize>,
    pub(crate) scroll: usize,
}

pub(crate) fn select_paint_info(node: usize) -> Option<SelPaint> {
    let snap = SEL_SNAP.with(|m| m.borrow().get(&node).cloned())?;
    let live = SEL_LIVE.with(|m| m.borrow().get(&node).cloned()).unwrap_or_default();
    let selected = resolve_selection(&snap.opts, snap.multiple, snap.listbox, live.sel.as_deref());
    Some(SelPaint { snap, selected, scroll: live.scroll })
}

impl SelPaint {
    /// The text the closed select shows.
    pub(crate) fn shown_text(&self) -> String {
        self.selected
            .first()
            .and_then(|i| self.snap.rows.iter().find(|r| r.opt == Some(*i)))
            .map(|r| r.label.clone())
            .unwrap_or_default()
    }
}

// ---- pure selection helpers -----------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Step {
    Prev,
    Next,
    First,
    Last,
    PagePrev(usize),
    PageNext(usize),
}

/// The option index `step` away from `cur`, skipping disabled options; stays
/// on `cur` when there is nowhere to go.
pub(crate) fn step_index(disabled: &[bool], cur: Option<usize>, step: Step) -> Option<usize> {
    let n = disabled.len();
    let enabled = |i: usize| !disabled[i];
    let next = |from: Option<usize>| -> Option<usize> {
        let start = from.map_or(0, |i| i + 1);
        (start..n).find(|i| enabled(*i)).or(from)
    };
    let prev = |from: Option<usize>| -> Option<usize> {
        let end = from.unwrap_or(n);
        (0..end).rev().find(|i| enabled(*i)).or(from)
    };
    match step {
        Step::Next => next(cur),
        Step::Prev => prev(cur),
        Step::First => (0..n).find(|i| enabled(*i)).or(cur),
        Step::Last => (0..n).rev().find(|i| enabled(*i)).or(cur),
        Step::PageNext(k) => (0..k).fold(cur, |c, _| next(c)),
        Step::PagePrev(k) => (0..k).fold(cur, |c, _| prev(c)),
    }
}

/// Type-ahead: the enabled option whose label starts with `buf` (case
/// insensitive, leading space ignored). One repeated letter cycles through the
/// matches after `cur`; a longer prefix searches from `cur` itself.
pub(crate) fn typeahead_find(opts: &[(String, bool)], cur: Option<usize>, buf: &str) -> Option<usize> {
    let b = buf.to_lowercase();
    let first = b.chars().next()?;
    let cycle = b.chars().all(|c| c == first);
    let key: String = if cycle { first.to_string() } else { b };
    let n = opts.len();
    if n == 0 {
        return None;
    }
    let from = match (cycle, cur) {
        (true, Some(c)) => c + 1,
        (false, Some(c)) => c,
        (_, None) => 0,
    };
    (0..n).map(|k| (from + k) % n).find(|i| !opts[*i].1 && opts[*i].0.trim_start().to_lowercase().starts_with(&key))
}

/// The type-ahead buffer: letters typed within a second of each other.
#[derive(Clone, Debug, Default)]
pub(crate) struct TypeAhead {
    pub(crate) buf: String,
    at: f64,
}

impl TypeAhead {
    pub(crate) fn push(&mut self, c: char, now: f64) -> &str {
        if now - self.at > 1.0 {
            self.buf.clear();
        }
        self.buf.push(c);
        self.at = now;
        &self.buf
    }
}

/// The new selection after a click on option `clicked` of a listbox: plain =
/// just it; Ctrl (multiple) toggles it; Shift (multiple) selects the range from
/// `anchor` (disabled options skipped).
pub(crate) fn click_selection(
    multiple: bool,
    current: &[usize],
    anchor: Option<usize>,
    clicked: usize,
    ctrl: bool,
    shift: bool,
    disabled: &[bool],
) -> Vec<usize> {
    if !multiple {
        return vec![clicked];
    }
    if shift {
        let a = anchor.unwrap_or(clicked);
        let (lo, hi) = (a.min(clicked), a.max(clicked));
        return (lo..=hi).filter(|i| !disabled.get(*i).copied().unwrap_or(true)).collect();
    }
    if ctrl {
        let mut v = current.to_vec();
        if let Some(p) = v.iter().position(|x| *x == clicked) {
            v.remove(p);
        } else {
            v.push(clicked);
        }
        return v;
    }
    vec![clicked]
}

// ---- the popup: pure state machine + geometry --------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PopupEvent {
    /// Click, Space/Enter or Alt-Down on a closed select.
    Activate,
    Escape,
    ClickAway,
    Blur,
    Tab,
    Choose,
}

/// Whether the popup is open after `e` (the whole open/close state machine).
pub(crate) fn popup_next_open(open: bool, e: PopupEvent) -> bool {
    match (open, e) {
        (false, PopupEvent::Activate) => true,
        (true, PopupEvent::Activate) => false,
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PopupState {
    pub(crate) node: usize,
    /// The highlighted option index.
    pub(crate) highlight: Option<usize>,
    /// First visible row.
    pub(crate) scroll: usize,
    pub(crate) ahead: TypeAhead,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PopupKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Enter,
    Space,
    Escape,
    Tab,
    Char(char),
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PopupAct {
    Stay,
    Choose(usize),
    Close,
    /// Close, and let the key (Tab) continue to its normal handling.
    CloseAndPass,
}

pub(crate) fn opt_disabled(rows: &[SelRow]) -> Vec<bool> {
    rows.iter().filter(|r| r.opt.is_some()).map(|r| r.disabled).collect()
}

pub(crate) fn row_of_opt(rows: &[SelRow], opt: usize) -> usize {
    rows.iter().position(|r| r.opt == Some(opt)).unwrap_or(0)
}

/// Scrolls the popup so that the highlighted option's row is visible.
pub(crate) fn popup_ensure_visible(st: &mut PopupState, rows: &[SelRow], visible: usize) {
    let visible = visible.max(1);
    if let Some(h) = st.highlight {
        let row = row_of_opt(rows, h);
        if row < st.scroll {
            st.scroll = row;
            // Show the group label above the first option of a group.
            if row > 0 && rows[row - 1].opt.is_none() {
                st.scroll = row - 1;
            }
        } else if row >= st.scroll + visible {
            st.scroll = row + 1 - visible;
        }
    }
    st.scroll = st.scroll.min(rows.len().saturating_sub(visible));
}

/// One key in an open popup. Moves the highlight, type-ahead, picks or closes.
pub(crate) fn popup_key(st: &mut PopupState, rows: &[SelRow], visible: usize, key: PopupKey, now: f64) -> PopupAct {
    let dis = opt_disabled(rows);
    let step = |st: &mut PopupState, s: Step| {
        st.highlight = step_index(&dis, st.highlight, s);
    };
    let page = visible.max(2) - 1;
    match key {
        PopupKey::Escape => return PopupAct::Close,
        PopupKey::Tab => return PopupAct::CloseAndPass,
        PopupKey::Enter | PopupKey::Space => {
            return match st.highlight {
                Some(h) if !dis.get(h).copied().unwrap_or(true) => PopupAct::Choose(h),
                _ => PopupAct::Close,
            };
        }
        PopupKey::Up => step(st, Step::Prev),
        PopupKey::Down => step(st, Step::Next),
        PopupKey::Home => step(st, Step::First),
        PopupKey::End => step(st, Step::Last),
        PopupKey::PageUp => step(st, Step::PagePrev(page)),
        PopupKey::PageDown => step(st, Step::PageNext(page)),
        PopupKey::Char(c) => {
            let labels: Vec<(String, bool)> =
                rows.iter().filter(|r| r.opt.is_some()).map(|r| (r.label.clone(), r.disabled)).collect();
            let buf = st.ahead.push(c, now).to_string();
            if let Some(i) = typeahead_find(&labels, st.highlight, &buf) {
                st.highlight = Some(i);
            }
        }
        PopupKey::Other => return PopupAct::Stay,
    }
    popup_ensure_visible(st, rows, visible);
    PopupAct::Stay
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PopupGeom {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
    pub(crate) row_h: f32,
    /// Visible rows (the rest scroll).
    pub(crate) visible: usize,
    pub(crate) above: bool,
}

/// Where the popup goes: below `anchor` (x, y, w, h in window space) when its
/// rows fit, else above when there is more room there (and shrunk to the room
/// when neither fits); at most `POPUP_MAX_ROWS` rows; at least the anchor's
/// width, `content_w` wide when the labels need more; kept inside the window.
pub(crate) fn popup_geometry(anchor: (f32, f32, f32, f32), n_rows: usize, row_h: f32, content_w: f32, win: (f32, f32)) -> PopupGeom {
    let (ax, ay, aw, ah) = anchor;
    let frame = 2.0;
    let want = n_rows.clamp(1, POPUP_MAX_ROWS);
    let fits = |room: f32| (((room - frame) / row_h).floor().max(0.0)) as usize;
    let below = fits(win.1 - (ay + ah));
    let above_rows = fits(ay);
    let (above, visible) = if below >= want {
        (false, want)
    } else if above_rows > below {
        (true, want.min(above_rows).max(1))
    } else {
        (false, want.min(below).max(1))
    };
    let h = visible as f32 * row_h + frame;
    let y = if above { ay - h } else { ay + ah };
    let y = y.max(0.0);
    let w = aw.max(content_w).min(win.0).max(1.0);
    let x = if ax + w > win.0 { (win.0 - w).max(0.0) } else { ax.max(0.0) };
    PopupGeom { x, y, w, h, row_h, visible, above }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PopupHit {
    /// A row index (into the rows list).
    Row(usize),
    Border,
    Outside,
}

pub(crate) fn popup_hit(g: &PopupGeom, scroll: usize, n_rows: usize, pos: (f32, f32)) -> PopupHit {
    let (px, py) = pos;
    if px < g.x || px >= g.x + g.w || py < g.y || py >= g.y + g.h {
        return PopupHit::Outside;
    }
    let ry = py - g.y - 1.0;
    if ry < 0.0 || px < g.x + 1.0 || px >= g.x + g.w - 1.0 {
        return PopupHit::Border;
    }
    let row = scroll + (ry / g.row_h) as usize;
    if row < n_rows && ((ry / g.row_h) as usize) < g.visible {
        PopupHit::Row(row)
    } else {
        PopupHit::Border
    }
}

/// Everything the popup needs to be painted and hit-tested, computed afresh
/// from the live DOM and the laid-out select each time.
pub(crate) struct PopupView {
    pub(crate) geom: PopupGeom,
    pub(crate) rows: Vec<SelRow>,
    pub(crate) selected: Vec<usize>,
    pub(crate) style: ComputedStyle,
    pub(crate) select: Handle,
}

/// The border box of node `node` in window space.
pub(crate) fn node_window_rect(laid: &Laid, cache: &ShapeCache, node: usize, scroll_offset: f32) -> Option<(f32, f32, f32, f32)> {
    let g = node_geometry(laid, cache, node);
    g.rects.first().map(|r| (r.0, r.1 - scroll_offset, r.2, r.3))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn popup_view(
    st: &PopupState,
    document: &Handle,
    laid: &Laid,
    cache: &ShapeCache,
    scroll_offset: f32,
    win: (f32, f32),
) -> Option<PopupView> {
    let select = find_node_by_id(document, st.node)?;
    let l = find_laid_node(laid, st.node)?;
    let style = l.style;
    let anchor = node_window_rect(laid, cache, st.node, scroll_offset)?;
    let rows = select_rows(&select);
    if rows.is_empty() {
        return None;
    }
    let row_h = line_h(&style) + POPUP_ROW_PAD;
    let widest = rows.iter().map(|r| r.label.chars().count()).max().unwrap_or(0) as f32;
    let bar = if rows.len() > POPUP_MAX_ROWS { POPUP_SCROLLBAR_W } else { 0.0 };
    let content_w = widest * avg_char_w(&style, false) + 2.0 * POPUP_TEXT_PAD + POPUP_GROUP_INDENT * 0.0 + bar + 2.0;
    let geom = popup_geometry(anchor, rows.len(), row_h, content_w, win);
    let selected = selected_indices(&select);
    Some(PopupView { geom, rows, selected, style, select })
}

// ---- painting -------------------------------------------------------------

fn rgb3(c: (u8, u8, u8)) -> TextColor {
    TextColor::rgb(c.0, c.1, c.2)
}

#[allow(clippy::too_many_arguments)]
fn paint_label(
    s: &ComputedStyle,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    text: &str,
    color: TextColor,
    bold: bool,
    x: f32,
    row_y: f32,
    row_h: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    if text.is_empty() || color.a() == 0 {
        return;
    }
    let mut st = *s;
    if bold {
        st.font_weight = cosmic_text::Weight::BOLD;
    }
    let (mut buffer, line_h) = shape_control(font_system, &st, text, color, false, 0.0);
    let (backgrounds, valigns, hidden, decos) = ([None], [VerticalAlign::default()], [false], [DecorationStyle::default()]);
    let top = row_y + ((row_h - line_h) / 2.0).max(0.0).floor();
    paint_buffer(&mut buffer, font_system, swash_cache, &backgrounds, &valigns, &hidden, &decos, pixels, win_w, win_h, x, top, clip, 0.0);
}

/// The closed `<select>` (a text line + arrow) or listbox, painted from the live
/// selection. All the look is here, easy to redesign.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_select_control(
    s: &ComputedStyle,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    clip: Option<(f32, f32, f32, f32)>,
) {
    let Some(info) = select_paint_info(s.node) else { return };
    let (cx, cy, cw, ch) = control_content_box(s, x, y, w, h);
    if cw <= 0.0 || ch <= 0.0 {
        return;
    }
    if s.select_widget & SEL_LIST != 0 {
        paint_listbox(s, &info, font_system, swash_cache, pixels, win_w, win_h, (cx, cy, cw, ch), clip);
    } else {
        paint_dropdown_face(s, &info, font_system, swash_cache, pixels, win_w, win_h, (cx, cy, cw, ch), clip);
    }
}

/// The text of the selected option, clipped before the arrow, and the arrow.
#[allow(clippy::too_many_arguments)]
fn paint_dropdown_face(
    s: &ComputedStyle,
    info: &SelPaint,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    (cx, cy, cw, ch): (f32, f32, f32, f32),
    clip: Option<(f32, f32, f32, f32)>,
) {
    let arrow_w = if s.appearance_none { 0.0 } else { SELECT_ARROW_W };
    let text_clip = Some(intersect_clip(cx, cy, (cw - arrow_w).max(0.0), ch, clip));
    paint_label(s, font_system, swash_cache, pixels, win_w, win_h, &info.shown_text(), s.color, false, cx, cy, ch, text_clip);
    if s.appearance_none || s.color.a() == 0 {
        return;
    }
    // A chevron: two strokes meeting at the bottom.
    let (mx, my) = (cx + cw - arrow_w / 2.0 - 1.0, cy + ch / 2.0 + 1.0);
    chevron(pixels, win_w, win_h, mx, my, s.color, clip);
}

/// The 8x4 drop chevron centred on (mx, my).
fn chevron(pixels: &mut [u32], win_w: u32, win_h: u32, mx: f32, my: f32, color: TextColor, clip: Option<(f32, f32, f32, f32)>) {
    for i in 0..=4 {
        let f = i as f32;
        for (px, py) in [(mx - 4.0 + f, my - 2.0 + f - 0.0), (mx + 4.0 - f, my - 2.0 + f)] {
            fill_rect(pixels, win_w, win_h, px.round(), (py - 1.0).round(), 1.0, 2.0, color, clip);
        }
    }
}

/// A listbox: rows from the scroll position, selected rows highlighted.
#[allow(clippy::too_many_arguments)]
fn paint_listbox(
    s: &ComputedStyle,
    info: &SelPaint,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    (cx, cy, cw, ch): (f32, f32, f32, f32),
    clip: Option<(f32, f32, f32, f32)>,
) {
    let focused = interaction_has(|st| st.focus == Some(s.node));
    let lh = line_h(s);
    let area = Some(intersect_clip(cx, cy, cw, ch, clip));
    let rows = &info.snap.rows;
    let visible = (ch / lh).ceil() as usize;
    for (n, row) in rows.iter().enumerate().skip(info.scroll).take(visible) {
        let ry = cy + (n - info.scroll) as f32 * lh;
        let selected = row.opt.is_some_and(|o| info.selected.contains(&o));
        let (bg, mut fg) = if selected && focused {
            (Some(rgb3(ACCENT)), rgb3(POPUP_HILITE_TEXT))
        } else if selected {
            (Some(rgb3(LIST_SELECTED_BLURRED_BG)), s.color)
        } else {
            (None, s.color)
        };
        if row.disabled && !selected {
            fg = rgb3(if row.opt.is_none() { POPUP_HEADER_TEXT } else { POPUP_DISABLED_TEXT });
        }
        if let Some(bg) = bg {
            fill_rect(pixels, win_w, win_h, cx, ry, cw, lh, bg, area);
        }
        let indent = if row.grouped { POPUP_GROUP_INDENT } else { 0.0 };
        paint_label(s, font_system, swash_cache, pixels, win_w, win_h, &row.label, fg, row.opt.is_none(), cx + 2.0 + indent, ry, lh, area);
    }
}

/// The dropdown overlay: shadow, frame, rows (highlight, selected, disabled,
/// group labels) and a scrollbar when it scrolls. Painted last, over the page.
pub(crate) fn paint_select_popup(
    view: &PopupView,
    st: &PopupState,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
) {
    let g = &view.geom;
    let s = &view.style;
    fill_rect(pixels, win_w, win_h, g.x + 1.0, g.y + 2.0, g.w, g.h, TextColor::rgba(0, 0, 0, 45), None);
    fill_rect(pixels, win_w, win_h, g.x, g.y, g.w, g.h, rgb3(POPUP_BORDER), None);
    fill_rect(pixels, win_w, win_h, g.x + 1.0, g.y + 1.0, g.w - 2.0, g.h - 2.0, rgb3(POPUP_BG), None);
    let inner = Some((g.x + 1.0, g.y + 1.0, g.w - 2.0, g.h - 2.0));
    let scrolls = view.rows.len() > g.visible;
    let text_w_clip = if scrolls { Some((g.x + 1.0, g.y + 1.0, g.w - 2.0 - POPUP_SCROLLBAR_W, g.h - 2.0)) } else { inner };
    for (n, row) in view.rows.iter().enumerate().skip(st.scroll).take(g.visible) {
        let ry = g.y + 1.0 + (n - st.scroll) as f32 * g.row_h;
        let hi = row.opt.is_some() && row.opt == st.highlight;
        let selected = row.opt.is_some_and(|o| view.selected.contains(&o));
        let mut fg = s.color;
        if hi {
            fill_rect(pixels, win_w, win_h, g.x + 1.0, ry, g.w - 2.0, g.row_h, rgb3(ACCENT), inner);
            fg = rgb3(POPUP_HILITE_TEXT);
        } else if selected {
            fill_rect(pixels, win_w, win_h, g.x + 1.0, ry, g.w - 2.0, g.row_h, rgb3(POPUP_SELECTED_BG), inner);
        }
        if row.disabled && !hi {
            fg = rgb3(if row.opt.is_none() { POPUP_HEADER_TEXT } else { POPUP_DISABLED_TEXT });
        }
        if fg.a() == 0 {
            fg = TextColor::rgb(0, 0, 0);
        }
        let indent = if row.grouped { POPUP_GROUP_INDENT } else { 0.0 };
        paint_label(
            s, font_system, swash_cache, pixels, win_w, win_h, &row.label, fg, row.opt.is_none(),
            g.x + 1.0 + POPUP_TEXT_PAD + indent, ry, g.row_h, text_w_clip,
        );
    }
    if scrolls {
        let track_h = g.h - 2.0;
        let total = view.rows.len() as f32;
        let thumb_h = (track_h * g.visible as f32 / total).max(12.0);
        let thumb_y = g.y + 1.0 + (track_h - thumb_h) * st.scroll as f32 / (total - g.visible as f32).max(1.0);
        let bx = g.x + g.w - 1.0 - POPUP_SCROLLBAR_W;
        fill_rect(pixels, win_w, win_h, bx, g.y + 1.0, POPUP_SCROLLBAR_W, track_h, TextColor::rgb(240, 240, 240), inner);
        fill_rect(pixels, win_w, win_h, bx + 1.0, thumb_y, POPUP_SCROLLBAR_W - 2.0, thumb_h, TextColor::rgb(170, 170, 170), inner);
    }
}

// ---- the App glue -----------------------------------------------------------

/// Seconds since the first call (type-ahead timing).
fn now_secs() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

fn popup_key_of(key: &Key, m: ModifiersState) -> PopupKey {
    match key {
        Key::Named(NamedKey::ArrowUp) => PopupKey::Up,
        Key::Named(NamedKey::ArrowDown) => PopupKey::Down,
        Key::Named(NamedKey::Home) => PopupKey::Home,
        Key::Named(NamedKey::End) => PopupKey::End,
        Key::Named(NamedKey::PageUp) => PopupKey::PageUp,
        Key::Named(NamedKey::PageDown) => PopupKey::PageDown,
        Key::Named(NamedKey::Enter) => PopupKey::Enter,
        Key::Named(NamedKey::Space) => PopupKey::Space,
        Key::Named(NamedKey::Escape) => PopupKey::Escape,
        Key::Named(NamedKey::Tab) => PopupKey::Tab,
        Key::Character(s) if !m.control_key() && !m.alt_key() && !m.super_key() => {
            let mut it = s.chars();
            match (it.next(), it.next()) {
                (Some(c), None) if !c.is_control() => PopupKey::Char(c),
                _ => PopupKey::Other,
            }
        }
        _ => PopupKey::Other,
    }
}

/// An App-level open popup.
pub(crate) struct SelectPopup {
    pub(crate) st: PopupState,
}

impl App {
    fn select_redraw(&mut self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// The focused element when it is an enabled `<select>`.
    pub(crate) fn focused_select(&self) -> Option<Handle> {
        let h = find_node_by_id(&self.document, self.interaction.focus?)?;
        (element_tag(&h) == "select" && !is_disabled_control(&h)).then_some(h)
    }

    pub(crate) fn select_popup_view(&self) -> Option<PopupView> {
        let p = self.select_popup.as_ref()?;
        let laid = self.cached.as_ref()?;
        popup_view(&p.st, &self.document, laid, &self.shape_cache, self.scroll_offset, { let (w, h) = self.page_size(); (w as f32, h as f32) })
    }

    pub(crate) fn select_popup_close(&mut self, why: PopupEvent) {
        let open = self.select_popup.is_some();
        if open && !popup_next_open(open, why) {
            self.select_popup = None;
            self.select_redraw();
        }
    }

    /// Opens the dropdown of a single `<select>` with the selected option highlighted.
    pub(crate) fn select_popup_open(&mut self, h: &Handle) {
        if is_listbox(h) || is_disabled_control(h) || !popup_next_open(self.select_popup.is_some(), PopupEvent::Activate) {
            return;
        }
        let rows = select_rows(h);
        let dis = opt_disabled(&rows);
        let highlight = selected_indices(h).first().copied().or_else(|| step_index(&dis, None, Step::First));
        let st = PopupState { node: node_id(h), highlight, scroll: 0, ahead: TypeAhead::default() };
        self.select_popup = Some(SelectPopup { st });
        if let Some(view) = self.select_popup_view() {
            let p = self.select_popup.as_mut().unwrap();
            popup_ensure_visible(&mut p.st, &view.rows, view.geom.visible);
        }
        self.select_redraw();
    }

    /// The user picked `idx` in `h`: selection changes, then `input` and `change` bubble.
    pub(crate) fn select_commit(&mut self, h: &Handle, idx: &[usize]) {
        if set_selected_indices(h, idx) {
            self.check_state_changed();
            let ev = [("bubbles", EventField::Bool(true))];
            self.js_dispatch(Some(h), "input", &ev);
            self.js_dispatch(Some(h), "change", &ev);
        }
        self.select_redraw();
    }

    /// Keys while the popup is open: it owns them all (nothing reaches the vim
    /// keys). Returns false only for Tab, which closes it and then moves focus.
    pub(crate) fn select_popup_key(&mut self, key: &Key) -> bool {
        let Some(view) = self.select_popup_view() else {
            self.select_popup = None;
            return false;
        };
        let pk = popup_key_of(key, self.modifiers);
        let p = self.select_popup.as_mut().unwrap();
        let act = popup_key(&mut p.st, &view.rows, view.geom.visible, pk, now_secs());
        match act {
            PopupAct::Stay => self.select_redraw(),
            PopupAct::Choose(i) => {
                self.select_popup_close(PopupEvent::Choose);
                self.select_commit(&view.select, &[i]);
            }
            PopupAct::Close => self.select_popup_close(PopupEvent::Escape),
            PopupAct::CloseAndPass => {
                self.select_popup_close(PopupEvent::Tab);
                return false;
            }
        }
        true
    }

    /// A press while the popup is open (consumed: the page never sees it, and
    /// the matching release is swallowed). Returns false when nothing is open.
    pub(crate) fn select_popup_press(&mut self, pos: (f32, f32)) -> bool {
        if self.select_popup.is_none() {
            return false;
        }
        let Some(view) = self.select_popup_view() else {
            self.select_popup = None;
            return false;
        };
        let scroll = self.select_popup.as_ref().unwrap().st.scroll;
        match popup_hit(&view.geom, scroll, view.rows.len(), pos) {
            PopupHit::Row(r) => {
                let row = &view.rows[r];
                if let Some(i) = row.opt.filter(|_| !row.disabled) {
                    self.select_popup_close(PopupEvent::Choose);
                    self.select_commit(&view.select, &[i]);
                }
            }
            PopupHit::Border => {}
            PopupHit::Outside => self.select_popup_close(PopupEvent::ClickAway),
        }
        self.swallow_release = true;
        true
    }

    /// Pointer motion over the open popup highlights the row under it.
    /// Returns whether the pointer is over the popup (the page gets no hover then).
    pub(crate) fn select_popup_hover(&mut self, pos: (f32, f32)) -> bool {
        let Some(view) = self.select_popup_view() else { return false };
        let scroll = self.select_popup.as_ref().unwrap().st.scroll;
        let hit = popup_hit(&view.geom, scroll, view.rows.len(), pos);
        if let PopupHit::Row(r) = hit {
            let row = &view.rows[r];
            if let Some(i) = row.opt.filter(|_| !row.disabled) {
                let p = self.select_popup.as_mut().unwrap();
                if p.st.highlight != Some(i) {
                    p.st.highlight = Some(i);
                    self.select_redraw();
                }
            }
        }
        hit != PopupHit::Outside
    }

    /// Wheel over the open popup scrolls its rows. Returns whether it did.
    pub(crate) fn select_popup_wheel(&mut self, dy: f32) -> bool {
        let Some(pos) = self.last_mouse_pos else { return false };
        let Some(view) = self.select_popup_view() else { return false };
        let p = self.select_popup.as_mut().unwrap();
        if popup_hit(&view.geom, p.st.scroll, view.rows.len(), pos) == PopupHit::Outside {
            return false;
        }
        let max = view.rows.len().saturating_sub(view.geom.visible);
        let delta = (-dy / view.geom.row_h).round() as i64;
        p.st.scroll = (p.st.scroll as i64 + delta).clamp(0, max as i64) as usize;
        self.select_redraw();
        true
    }

    /// A click (release) on select `h`: a dropdown opens; a listbox picks the row.
    pub(crate) fn select_click(&mut self, h: &Handle, pos: (f32, f32)) {
        if is_disabled_control(h) {
            return;
        }
        // A label forwarding its click here carries a position outside the select.
        let under = self.element_at(pos);
        if !self.chain_for(under).contains(&node_id(h)) {
            return;
        }
        if !is_listbox(h) {
            self.select_popup_open(h);
            return;
        }
        let Some(laid) = self.cached.as_ref() else { return };
        let node = node_id(h);
        let (Some(rect), Some(l)) = (node_window_rect(laid, &self.shape_cache, node, self.scroll_offset), find_laid_node(laid, node)) else {
            return;
        };
        let s = l.style;
        let (_, cy, _, _) = control_content_box(&s, rect.0, rect.1, rect.2, rect.3);
        let scroll = SEL_LIVE.with(|m| m.borrow().get(&node).map_or(0, |l| l.scroll));
        let row = scroll + ((pos.1 - cy) / line_h(&s)).floor().max(0.0) as usize;
        let rows = select_rows(h);
        let Some(opt) = rows.get(row).and_then(|r| r.opt.filter(|_| !r.disabled)) else { return };
        let (ctrl, shift) = (self.modifiers.control_key(), self.modifiers.shift_key());
        self.listbox_pick(h, opt, ctrl, shift);
    }

    fn listbox_pick(&mut self, h: &Handle, opt: usize, ctrl: bool, shift: bool) {
        let rows = select_rows(h);
        let dis = opt_disabled(&rows);
        let multiple = is_multiple(h);
        let node = node_id(h);
        let (cur, anchor) = (selected_indices(h), SEL_LIVE.with(|m| m.borrow().get(&node).and_then(|l| l.anchor)));
        let new = click_selection(multiple, &cur, anchor, opt, ctrl, shift, &dis);
        SEL_LIVE.with(|m| {
            let mut m = m.borrow_mut();
            let l = m.entry(node).or_default();
            if !(multiple && shift) || l.anchor.is_none() {
                l.anchor = Some(opt);
            }
            l.cursor = Some(opt);
        });
        self.select_commit(h, &new);
        self.listbox_scroll_to(h, opt);
    }

    fn listbox_scroll_to(&mut self, h: &Handle, opt: usize) {
        let node = node_id(h);
        let Some(laid) = self.cached.as_ref() else { return };
        let Some(l) = find_laid_node(laid, node) else { return };
        let s = l.style;
        let (_, _, _, ch) = control_content_box(&s, l.x, l.y, l.w, l.h);
        let visible = ((ch / line_h(&s)).floor() as usize).max(1);
        let rows = select_rows(h);
        let row = row_of_opt(&rows, opt);
        SEL_LIVE.with(|m| {
            let mut m = m.borrow_mut();
            let l = m.entry(node).or_default();
            if row < l.scroll {
                l.scroll = row;
            } else if row >= l.scroll + visible {
                l.scroll = row + 1 - visible;
            }
            l.scroll = l.scroll.min(rows.len().saturating_sub(visible));
        });
    }

    /// A key for the focused enabled select while no popup is open. Returns
    /// whether it was consumed (false: it falls through to scratchy's keys).
    pub(crate) fn select_control_key(&mut self, h: &Handle, key: &Key) -> bool {
        let m = self.modifiers;
        let (ctrl, alt, shift) = (m.control_key(), m.alt_key(), m.shift_key());
        let listbox = is_listbox(h);
        let rows = select_rows(h);
        let dis = opt_disabled(&rows);
        let cur = if listbox {
            SEL_LIVE.with(|m| m.borrow().get(&node_id(h)).and_then(|l| l.cursor)).or_else(|| selected_indices(h).last().copied())
        } else {
            selected_indices(h).first().copied()
        };
        let step = match key {
            Key::Named(NamedKey::Escape) => {
                self.set_focus_element(None, false);
                return true;
            }
            Key::Named(NamedKey::Enter | NamedKey::F4) if !listbox => {
                self.select_popup_open(h);
                return true;
            }
            Key::Named(NamedKey::Space) if !listbox => {
                self.select_popup_open(h);
                return true;
            }
            Key::Named(NamedKey::Space) if is_multiple(h) => {
                if let Some(c) = cur {
                    self.listbox_pick(h, c, true, false);
                }
                return true;
            }
            Key::Named(NamedKey::ArrowDown) if alt && !listbox => {
                self.select_popup_open(h);
                return true;
            }
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowRight) if !ctrl && !alt => Step::Next,
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowLeft) if !ctrl && !alt => Step::Prev,
            Key::Named(NamedKey::Home) if !ctrl && !alt => Step::First,
            Key::Named(NamedKey::End) if !ctrl && !alt => Step::Last,
            Key::Named(NamedKey::PageDown) if !ctrl && !alt => Step::PageNext(self.select_page_rows(h)),
            Key::Named(NamedKey::PageUp) if !ctrl && !alt => Step::PagePrev(self.select_page_rows(h)),
            Key::Character(s) if !ctrl && !alt && !m.super_key() => {
                let mut it = s.chars();
                let (Some(c), None) = (it.next(), it.next()) else { return false };
                if c.is_control() {
                    return false;
                }
                let labels: Vec<(String, bool)> =
                    rows.iter().filter(|r| r.opt.is_some()).map(|r| (r.label.clone(), r.disabled)).collect();
                let buf = self.select_ahead.push(c, now_secs()).to_string();
                if let Some(i) = typeahead_find(&labels, cur, &buf) {
                    self.select_move_to(h, i, false);
                }
                return true;
            }
            _ => return false,
        };
        if let Some(i) = step_index(&dis, cur, step) {
            self.select_move_to(h, i, shift);
        }
        true
    }

    /// Visible rows of a listbox, or 10 for a dropdown (PageUp/PageDown).
    fn select_page_rows(&self, h: &Handle) -> usize {
        if !is_listbox(h) {
            return POPUP_MAX_ROWS;
        }
        let Some(l) = self.cached.as_ref().and_then(|laid| find_laid_node(laid, node_id(h))) else { return 4 };
        let s = l.style;
        let (_, _, _, ch) = control_content_box(&s, l.x, l.y, l.w, l.h);
        ((ch / line_h(&s)).floor() as usize).saturating_sub(1).max(1)
    }

    /// Keyboard selection of option `i` (a closed dropdown changes at once;
    /// a listbox moves its cursor, Shift extends a `multiple` range).
    fn select_move_to(&mut self, h: &Handle, i: usize, shift: bool) {
        if is_listbox(h) {
            self.listbox_pick(h, i, false, shift);
        } else {
            self.select_commit(h, &[i]);
        }
    }
}
