//! QEL2b: ResizeObserver sizes from the layout the rendering update already has.
//!
//! HTML "update the rendering" lays the document out once per frame and gathers the active resize
//! observations from THAT layout. Before QEL2b the RO step (`__scratchy_roRun`) asked for a JS-side
//! forced layout of its own (`gRect` -> `__d_rect`) and computed the content box from ~10
//! `getComputedStyle` reads per target (a second cascade). Now:
//!
//! - the JS side registers each observed target here (`track`, native `__d_ro_track`), so Rust knows
//!   which boxes to keep;
//! - every complete layout the page gets records the boxes of those targets: the App's own frame
//!   (`app_layout`, from `App::install_laid` and from the rendering update when the shown layout is
//!   current) and every forced layout (`forced_layout`, from `GeomCache::refresh`, engine or from
//!   scratch);
//! - `__d_ro_box` answers from the recorded boxes when they describe the DOM as it is now; only when
//!   they do not (a resize observer callback wrote the DOM inside the depth loop, a headless page whose
//!   step 6 did not lay out) does it run the forced layout, labelled as rendering-update work;
//! - an App layout whose target boxes differ from the last recorded ones bumps `epoch`, part of the
//!   JS observation signature, so a size change with no DOM write (image decode, CSS animation or
//!   transition, `:hover` restyle, viewport) asks for a rendering update;
//! - while the App has its frame's layout still to do (`set_wait`), the observers wait for it instead
//!   of forcing one (`__d_ro_wait`).
//!
//! The boxes are `[border w, border h, content w, content h, content x, content y]`: the outermost
//! `Laid` of the element (as `js_geom::walk` picks it); content = border box minus padding and border;
//! a non-replaced `display: inline` element and an element with no box report no box. The
//! inline/block swap of vertical writing modes stays in JS.
use super::*;

/// One observed target: the weak handle keeps the node's address unique while it is observed.
struct Tracked {
    _node: WeakHandle,
    count: u32,
    /// `img`, `video`, `input`, ...: an inline replaced element has a size.
    replaced: bool,
}

/// The target boxes recorded from one layout of one document.
struct Boxes {
    /// The document the layout is of (its node id).
    page: usize,
    /// `js_geom::render_stamp()` the layout was built at.
    stamp: crate::js_geom::RenderStamp,
    /// Forced layouts: the geometry cache key's (write generation, CSS versions); an in-place shift
    /// (QYTC1a) changes the render stamp but no size.
    gen_css: Option<(u64, (u64, u64))>,
    vp: (u32, u32),
    /// Tracked node -> its box (`None`: no box). A tracked node missing here was not tracked yet.
    map: HashMap<usize, Option<[f32; 6]>>,
}

#[derive(Default)]
struct State {
    tracked: HashMap<usize, Tracked>,
    boxes: Option<Boxes>,
    epoch: u64,
    wait: bool,
    stats: Stats,
}

/// `--perf-report` / frame-stats counters.
#[derive(Default, Clone, Copy)]
pub(crate) struct Stats {
    /// App layouts whose target boxes were recorded.
    pub(crate) app_collects: u64,
    /// Forced layouts whose target boxes were recorded.
    pub(crate) forced_collects: u64,
    /// Forced layouts run by `__d_ro_box` (the recorded boxes were stale).
    pub(crate) ro_layouts: u64,
    /// App layouts that changed a target box (epoch bumps).
    pub(crate) changes: u64,
    /// Rendering updates whose RO step gathered (`__scratchy_roRun` past its "anything new?" test).
    pub(crate) runs: u64,
    /// ResizeObserver callbacks invoked.
    pub(crate) callbacks: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

const REPLACED: &[&str] = &["img", "video", "canvas", "iframe", "object", "embed", "input", "select", "textarea", "button", "svg", "meter", "progress"];

/// A new document's engine starts: nothing is observed.
pub(crate) fn reset() {
    STATE.with(|s| {
        let stats = s.borrow().stats;
        *s.borrow_mut() = State { stats, ..State::default() };
    });
}

/// `observe()` (+1) / `unobserve()` and `disconnect()` (-1) of element `h`.
pub(crate) fn track(h: &Handle, delta: i32) {
    let id = node_id(h);
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if delta > 0 {
            let replaced = match &h.data {
                NodeData::Element { name, .. } => REPLACED.contains(&&*name.local),
                _ => false,
            };
            s.tracked.entry(id).or_insert(Tracked { _node: Rc::downgrade(h), count: 0, replaced }).count += 1;
        } else if let Some(t) = s.tracked.get_mut(&id) {
            t.count = t.count.saturating_sub(1);
            if t.count == 0 {
                s.tracked.remove(&id);
            }
        }
        if s.tracked.is_empty() {
            s.boxes = None;
            s.wait = false;
        }
    });
}

/// Something is observed (cheap: callers test this before any work).
pub(crate) fn active() -> bool {
    STATE.with(|s| !s.borrow().tracked.is_empty())
}

fn collect(l: &Laid, tracked: &HashMap<usize, Tracked>) -> HashMap<usize, Option<[f32; 6]>> {
    let mut map: HashMap<usize, Option<[f32; 6]>> = HashMap::with_capacity(tracked.len());
    walk(l, tracked, &mut map);
    for id in tracked.keys() {
        map.entry(*id).or_insert(None);
    }
    map
}

fn walk(l: &Laid, tracked: &HashMap<usize, Tracked>, map: &mut HashMap<usize, Option<[f32; 6]>>) {
    crate::stack::grow(|| walk_body(l, tracked, map)) // QSTK1x: recursive per nesting level
}

/// The RO box of `l`, the outermost box of a tracked element.
fn box_of(l: &Laid, t: &Tracked) -> Option<[f32; 6]> {
    let s = &l.style;
    if matches!(s.display, crate::css::Display::Inline) && !t.replaced && !matches!(l.content, LaidContent::Image(_)) {
        return None;
    }
    let (pl, pt) = (s.padding.left.px(), s.padding.top.px());
    let (pr, pb) = (s.padding.right.px(), s.padding.bottom.px());
    let bw = &s.border_width;
    let cw = (l.w - pl - pr - bw[3] - bw[1]).max(0.0);
    let ch = (l.h - pt - pb - bw[0] - bw[2]).max(0.0);
    Some([l.w, l.h, cw, ch, pl, pt])
}

fn walk_body(l: &Laid, tracked: &HashMap<usize, Tracked>, map: &mut HashMap<usize, Option<[f32; 6]>>) {
    if map.len() == tracked.len() {
        return;
    }
    let n = l.style.node;
    // The outermost box of an element wins (a wrapper may share its node), as in `js_geom::walk`.
    if n != 0 && l.style.pseudo == 0 && !map.contains_key(&n)
        && let Some(t) = tracked.get(&n)
    {
        map.insert(n, box_of(l, t));
    }
    match &l.content {
        LaidContent::Blocks(c) | LaidContent::TableRow(c) => c.iter().for_each(|c| walk(c, tracked, map)),
        LaidContent::MixedInline(items) => {
            for it in items {
                if let MixedPositioned::Embedded(b) = it {
                    walk(b, tracked, map);
                }
            }
        }
        LaidContent::Text(_) | LaidContent::Image(_) => {}
    }
    l.positioned.iter().for_each(|p| walk(p, tracked, map));
}

/// `App::install_laid`: the App finished a complete layout `l` of `page` for a `vw` x `vh` viewport,
/// built at `stamp`. Recorded (see `app_layout`) when the observers wait for it, when nothing is recorded,
/// for a new viewport, or when the DOM is as recorded (the layout changed without a DOM write: CSS
/// animation / transition, `:hover`). A frame for DOM writes is not walked here: the observation signature
/// moved with the write, and step 7 records the shown layout when it gathers.
pub(crate) fn app_install(page: &Handle, l: &Laid, stamp: crate::js_geom::RenderStamp, vw: u32, vh: u32) {
    let want = STATE.with(|s| {
        let s = s.borrow();
        !s.tracked.is_empty()
            && (s.wait || s.boxes.as_ref().is_none_or(|b| b.stamp == stamp || b.vp != (vw, vh) || b.page != node_id(page)))
    });
    if want {
        app_layout(page, l, stamp, vw, vh);
    } else {
        set_wait(false);
    }
}

/// The App's complete layout `l` of `page` for a `vw` x `vh` viewport, built at `stamp`: record the target
/// boxes; a box that differs from the last recorded one bumps the epoch (a rendering update is owed).
/// Ends a `set_wait`.
pub(crate) fn app_layout(page: &Handle, l: &Laid, stamp: crate::js_geom::RenderStamp, vw: u32, vh: u32) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.wait = false;
        if s.tracked.is_empty() {
            return;
        }
        let map = collect(l, &s.tracked);
        let pid = node_id(page);
        let same = s.boxes.as_ref().is_some_and(|b| b.page == pid && b.vp == (vw, vh) && b.map == map);
        if !same {
            s.epoch += 1;
            s.stats.changes += 1;
        }
        s.stats.app_collects += 1;
        s.boxes = Some(Boxes { page: pid, stamp, gen_css: None, vp: (vw, vh), map });
    });
}

thread_local! {
    /// A forced layout's rect walk (`js_geom::walk`) is recording the target boxes (`note`): no walk of
    /// its own.
    static COLLECTING: Cell<bool> = const { Cell::new(false) };
    static PENDING: RefCell<HashMap<usize, Option<[f32; 6]>>> = RefCell::new(HashMap::new());
}

/// A forced layout's rect walk is about to run: record the targets' boxes during it (true when anything
/// is observed; `end_forced` then stores them).
pub(crate) fn begin_forced() -> bool {
    let on = active();
    COLLECTING.with(|c| c.set(on));
    if on {
        PENDING.with(|p| p.borrow_mut().clear());
    }
    on
}

#[inline]
pub(crate) fn collecting() -> bool {
    COLLECTING.with(Cell::get)
}

/// `js_geom::walk` met the outermost box of an element (only while `collecting`).
pub(crate) fn note(l: &Laid) {
    let n = l.style.node;
    if l.style.pseudo != 0 {
        return;
    }
    STATE.with(|s| {
        if let Some(t) = s.borrow().tracked.get(&n) {
            PENDING.with(|p| p.borrow_mut().insert(n, box_of(l, t)));
        }
    });
}

/// The forced layout of `page` (geometry cache key `gen_css`) whose walk `begin_forced` started is done:
/// store the boxes it recorded.
pub(crate) fn end_forced(page: &Handle, gen_css: (u64, (u64, u64)), vw: u32, vh: u32) {
    COLLECTING.with(|c| c.set(false));
    let mut map = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.tracked.is_empty() {
            return;
        }
        for id in s.tracked.keys() {
            map.entry(*id).or_insert(None);
        }
        s.stats.forced_collects += 1;
        s.boxes = Some(Boxes { page: node_id(page), stamp: crate::js_geom::render_stamp(), gen_css: Some(gen_css), vp: (vw, vh), map });
    });
}

/// What the recorded boxes say about node `h` of `page` as the DOM is now.
pub(crate) enum Answer {
    Box([f32; 6]),
    NoBox,
    /// The recorded boxes are of another state (or do not cover `h`): a layout is needed.
    Stale,
}

pub(crate) fn lookup(page: &Handle, h: &Handle, vw: u32, vh: u32) -> Answer {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(b) = &s.boxes else { return Answer::Stale };
        let now = (crate::js_dom::geom_gen(), crate::js_style::css_ver());
        let fresh = b.stamp == crate::js_geom::render_stamp() || b.gen_css == Some(now);
        if b.page != node_id(page) || b.vp != (vw, vh) || !fresh {
            return Answer::Stale;
        }
        match b.map.get(&node_id(h)) {
            Some(Some(v)) => Answer::Box(*v),
            Some(None) => Answer::NoBox,
            None => Answer::Stale,
        }
    })
}

/// Are the recorded boxes those of the DOM as it is now, for every tracked node?
pub(crate) fn current(page: &Handle, vw: u32, vh: u32) -> bool {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(b) = &s.boxes else { return false };
        let now = (crate::js_dom::geom_gen(), crate::js_style::css_ver());
        (b.stamp == crate::js_geom::render_stamp() || b.gen_css == Some(now))
            && b.page == node_id(page)
            && b.vp == (vw, vh)
            && s.tracked.keys().all(|k| b.map.contains_key(k))
    })
}

/// The App's frame layout is still to come: the observers wait for it (true) / need not wait (false).
pub(crate) fn set_wait(on: bool) {
    STATE.with(|s| s.borrow_mut().wait = on);
}

pub(crate) fn waiting() -> bool {
    STATE.with(|s| s.borrow().wait)
}

pub(crate) fn epoch() -> u64 {
    STATE.with(|s| s.borrow().epoch)
}

pub(crate) fn note_ro_layout() {
    STATE.with(|s| s.borrow_mut().stats.ro_layouts += 1);
}

/// One RO step that gathered and invoked `callbacks` callbacks.
pub(crate) fn note_run(callbacks: u32) {
    STATE.with(|s| {
        let st = &mut s.borrow_mut().stats;
        st.runs += 1;
        st.callbacks += u64::from(callbacks);
    });
}

pub(crate) fn stats() -> Stats {
    STATE.with(|s| s.borrow().stats)
}

pub(crate) fn stats_json() -> String {
    let s = stats();
    format!(
        "{{\"runs\":{},\"callbacks\":{},\"app_collects\":{},\"forced_collects\":{},\"ro_layouts\":{},\"changes\":{}}}",
        s.runs, s.callbacks, s.app_collects, s.forced_collects, s.ro_layouts, s.changes
    )
}
