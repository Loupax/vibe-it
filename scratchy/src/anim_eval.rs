//! QAN1b: evaluation of CSS animations at a document time. The parsing and
//! the registry live in `animation.rs`; `resolve_style_for` calls
//! `evaluate` only for an element that declares `animation`.
//!
//! Method: for the keyframe interval the directed progress falls in, the
//! element is cascaded twice more with the interval's two keyframes'
//! declarations entered at `ANIM_KEY` (above normal declarations, below
//! `!important`), so every property keeps its existing parsing and unit
//! resolution; the two `ComputedStyle`s are then interpolated field by
//! field and every other field flips at 50 %.
use super::*;


struct StartEntry {
    start: f64,
    /// Keeps the node's allocation alive, so its address (the key) cannot
    /// be reused by another node while the entry exists.
    _node: std::rc::Weak<markup5ever_rcdom::Node>,
    seen: u64,
}

type StartKey = (usize, bool, usize, String);

thread_local! {
    static DOC_CLOCK: Cell<f64> = const { Cell::new(0.0) };
    static ANIM_STARTS: RefCell<HashMap<StartKey, StartEntry>> = RefCell::new(HashMap::new());
    static ANIM_PASS: Cell<u64> = const { Cell::new(1) };
    static APP_NAV: Cell<Option<(u64, std::time::Instant)>> = const { Cell::new(None) };
    /// QAN1c: test seam, a fixed app clock in ms (`None` = the real one).
    static APP_CLOCK_OVERRIDE: Cell<Option<f64>> = const { Cell::new(None) };
    /// QAN1c: the animations styled in the current pass / in the last finished pass.
    static ANIM_REPORT: RefCell<Vec<AnimRecord>> = const { RefCell::new(Vec::new()) };
    static ANIM_LAST: RefCell<Vec<AnimRecord>> = const { RefCell::new(Vec::new()) };
}

/// Identity of one running animation: node address, pseudo-element name,
/// index in the `animation` list, `@keyframes` name.
pub(crate) type AnimKey = (usize, Option<String>, usize, String);

/// One animation styled by a build (QAN1c): what the App needs to time
/// its start, iteration and end events.
#[derive(Clone)]
pub(crate) struct AnimRecord {
    pub(crate) key: AnimKey,
    pub(crate) node: std::rc::Weak<markup5ever_rcdom::Node>,
    pub(crate) anim: Animation,
    /// Document clock (ms) at which the animation started.
    pub(crate) start: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AnimPhase {
    Before,
    Running,
    After,
}

/// Where an animation is `elapsed` ms after its start.
pub(crate) fn phase_of(a: &Animation, elapsed: f64) -> AnimPhase {
    let active = elapsed - a.delay as f64;
    let (dur, iters) = (a.duration as f64, a.iterations as f64);
    if active < 0.0 {
        AnimPhase::Before
    } else if dur <= 0.0 || iters <= 0.0 || (iters.is_finite() && active >= dur * iters) {
        AnimPhase::After
    } else {
        AnimPhase::Running
    }
}

/// The animations the last finished build styled (replaced every pass).
pub(crate) fn take_animation_report() -> Vec<AnimRecord> {
    ANIM_LAST.with(|l| std::mem::take(&mut *l.borrow_mut()))
}

/// The document clock set for the current/last build.
pub(crate) fn document_clock_ms() -> f64 {
    DOC_CLOCK.with(|c| c.get())
}

/// Test seam: freeze the app clock at `ms` (`None` restores the real one).
#[cfg(test)]
pub(crate) fn set_app_clock_override(ms: Option<f64>) {
    APP_CLOCK_OVERRIDE.with(|c| c.set(ms));
}

/// Milliseconds since the current navigation's first build (0 before one).
pub(crate) fn app_clock_ms() -> f64 {
    if let Some(ms) = APP_CLOCK_OVERRIDE.with(|c| c.get()) {
        return ms;
    }
    APP_NAV.with(|n| n.get()).map_or(0.0, |(_, t)| t.elapsed().as_secs_f64() * 1000.0)
}

/// The document clock in milliseconds. The caller sets it before
/// `build_layout_tree_with_resources` (the WPT runner uses 0).
pub(crate) fn set_document_clock(ms: f64) {
    DOC_CLOCK.with(|c| c.set(ms));
}

/// App clock: milliseconds since the navigation `generation` first built
/// (navigation start for now; no frame loop, one evaluation per restyle).
pub(crate) fn set_app_clock(generation: u64) {
    if let Some(ms) = APP_CLOCK_OVERRIDE.with(|c| c.get()) {
        APP_NAV.with(|n| {
            if n.get().is_none_or(|(g, _)| g != generation) {
                n.set(Some((generation, std::time::Instant::now())));
                ANIM_STARTS.with(|s| s.borrow_mut().clear());
            }
        });
        set_document_clock(ms);
        return;
    }
    let now = std::time::Instant::now();
    let start = APP_NAV.with(|n| match n.get() {
        Some((g, t)) if g == generation => t,
        _ => {
            n.set(Some((generation, now)));
            ANIM_STARTS.with(|s| s.borrow_mut().clear());
            now
        }
    });
    set_document_clock(now.duration_since(start).as_secs_f64() * 1000.0);
}

/// Called after a tree build: forgets start times of animations that were
/// not styled in this pass (name changed, node gone).
pub(crate) fn finish_animation_pass() {
    let pass = ANIM_PASS.with(|p| p.get());
    ANIM_STARTS.with(|s| s.borrow_mut().retain(|_, e| e.seen == pass));
    ANIM_PASS.with(|p| p.set(pass + 1));
    let mut rep = ANIM_REPORT.with(|r| std::mem::take(&mut *r.borrow_mut()));
    let mut seen = std::collections::HashSet::new();
    rep.reverse();
    rep.retain(|r| seen.insert(r.key.clone()));
    rep.reverse();
    ANIM_LAST.with(|l| *l.borrow_mut() = rep);
}

fn start_time(handle: &Handle, pseudo: bool, idx: usize, name: &str) -> f64 {
    let now = DOC_CLOCK.with(|c| c.get());
    let pass = ANIM_PASS.with(|p| p.get());
    ANIM_STARTS.with(|s| {
        let mut s = s.borrow_mut();
        let key = (node_id(handle), pseudo, idx, name.to_string());
        let e = s.entry(key).or_insert_with(|| StartEntry { start: now, _node: Rc::downgrade(handle), seen: pass });
        e.seen = pass;
        e.start
    })
}

fn cubic(a: f32, b: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t
}

impl TimingFunction {
    /// Output progress for input progress `x`.
    pub(crate) fn ease(&self, x: f32) -> f32 {
        match *self {
            TimingFunction::Linear => x,
            TimingFunction::CubicBezier(x1, y1, x2, y2) => {
                if x <= 0.0 || x >= 1.0 {
                    return x.clamp(0.0, 1.0);
                }
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                let mut t = x;
                for _ in 0..40 {
                    let v = cubic(x1, x2, t);
                    if (v - x).abs() < 1e-6 {
                        break;
                    }
                    if v < x {
                        lo = t
                    } else {
                        hi = t
                    }
                    t = (lo + hi) / 2.0;
                }
                cubic(y1, y2, t)
            }
            TimingFunction::Steps(n, pos) => {
                let n = n.max(1) as f32;
                let mut step = (x * n).floor();
                if matches!(pos, StepPos::JumpStart | StepPos::JumpBoth) {
                    step += 1.0;
                }
                let jumps = match pos {
                    StepPos::JumpNone => (n - 1.0).max(1.0),
                    StepPos::JumpBoth => n + 1.0,
                    _ => n,
                };
                (step / jumps).clamp(0.0, 1.0)
            }
        }
    }
}

/// Where an animation is `elapsed` ms after its start: the directed
/// progress (0..=1) of the current iteration, or `None` when it has no
/// effect (before the start without backwards fill, after the end without
/// forwards fill).
pub(crate) fn directed_progress(a: &Animation, elapsed: f64) -> Option<f32> {
    let dur = a.duration as f64;
    let iters = a.iterations as f64;
    let active = elapsed - a.delay as f64;
    let active_dur = if iters.is_infinite() { f64::INFINITY } else { dur * iters };
    let backwards = matches!(a.fill, AnimFill::Backwards | AnimFill::Both);
    let forwards = matches!(a.fill, AnimFill::Forwards | AnimFill::Both);
    let (iter, simple) = if active < 0.0 {
        if !backwards {
            return None;
        }
        (0.0, 0.0)
    } else if active >= active_dur || dur <= 0.0 {
        if !forwards {
            return None;
        }
        if iters <= 0.0 {
            (0.0, 0.0)
        } else if iters.fract() == 0.0 {
            (iters - 1.0, 1.0)
        } else {
            (iters.floor(), iters.fract())
        }
    } else {
        let overall = active / dur;
        (overall.floor(), overall.fract())
    };
    let rev = match a.direction {
        AnimDirection::Normal => false,
        AnimDirection::Reverse => true,
        AnimDirection::Alternate => iter % 2.0 == 1.0,
        AnimDirection::AlternateReverse => iter % 2.0 == 0.0,
    };
    let s = simple as f32;
    Some(if rev { 1.0 - s } else { s })
}

type Ends<'k> = (f32, Vec<&'k Keyframe>, f32, Vec<&'k Keyframe>);

/// The keyframe interval containing progress `p`: (from offset, from
/// keyframes, to offset, to keyframes). An empty list is the underlying
/// (base) style (a missing 0 % / 100 %).
fn interval(kf: &[Keyframe], p: f32) -> Ends<'_> {
    let (lo, hi) = if p >= 1.0 {
        (kf.iter().map(|k| k.offset).filter(|&o| o < 1.0).fold(0.0, f32::max), 1.0)
    } else {
        let lo = kf.iter().map(|k| k.offset).filter(|&o| o <= p).fold(0.0, f32::max);
        let hi = kf.iter().map(|k| k.offset).filter(|&o| o > p).fold(1.0, f32::min);
        (lo, hi)
    };
    let a = kf.iter().filter(|k| k.offset == lo).collect();
    let b = kf.iter().filter(|k| k.offset == hi).collect();
    (lo, a, hi, b)
}

pub(crate) trait Interp: Copy {
    /// `None` when the two values do not interpolate (discrete).
    fn interp(a: &Self, b: &Self, t: f32) -> Option<Self>;
    fn same(a: &Self, b: &Self) -> bool;
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl Interp for f32 {
    fn interp(a: &f32, b: &f32, t: f32) -> Option<f32> {
        Some(lerp(*a, *b, t))
    }
    fn same(a: &f32, b: &f32) -> bool {
        a == b
    }
}

impl Interp for TextColor {
    /// Premultiplied sRGB.
    fn interp(a: &TextColor, b: &TextColor, t: f32) -> Option<TextColor> {
        let pa = a.a() as f32 / 255.0;
        let pb = b.a() as f32 / 255.0;
        let alpha = lerp(pa, pb, t).clamp(0.0, 1.0);
        let ch = |x: u8, y: u8| {
            if alpha <= 0.0 {
                return 0.0;
            }
            (lerp(x as f32 * pa, y as f32 * pb, t) / alpha).clamp(0.0, 255.0)
        };
        Some(TextColor::rgba(
            ch(a.r(), b.r()).round() as u8,
            ch(a.g(), b.g()).round() as u8,
            ch(a.b(), b.b()).round() as u8,
            (alpha * 255.0).round() as u8,
        ))
    }
    fn same(a: &TextColor, b: &TextColor) -> bool {
        a == b
    }
}

impl Interp for BoxLength {
    fn interp(a: &BoxLength, b: &BoxLength, t: f32) -> Option<BoxLength> {
        match (a, b) {
            (BoxLength::Px(x), BoxLength::Px(y)) => Some(BoxLength::Px(lerp(*x, *y, t))),
            (BoxLength::Percent(x), BoxLength::Percent(y)) => Some(BoxLength::Percent(lerp(*x, *y, t))),
            _ => None,
        }
    }
    fn same(a: &BoxLength, b: &BoxLength) -> bool {
        match (a, b) {
            (BoxLength::Px(x), BoxLength::Px(y)) | (BoxLength::Percent(x), BoxLength::Percent(y)) => x == y,
            (BoxLength::Calc(p, ..), BoxLength::Calc(q, ..)) => std::ptr::eq(*p, *q),
            _ => false,
        }
    }
}

impl Interp for i32 {
    fn interp(a: &i32, b: &i32, t: f32) -> Option<i32> {
        Some(lerp(*a as f32, *b as f32, t).round() as i32)
    }
    fn same(a: &i32, b: &i32) -> bool {
        a == b
    }
}

fn axis_lerp(a: TranslateAxis, b: TranslateAxis, t: f32) -> TranslateAxis {
    TranslateAxis { px: lerp(a.px, b.px, t), frac: lerp(a.frac, b.frac, t) }
}

const ZERO_AXIS: TranslateAxis = TranslateAxis { px: 0.0, frac: 0.0 };

/// `Option<T>`: both set interpolate; both unset stay unset; one unset is
/// discrete.
fn opt_interp<T: Interp>(a: &Option<T>, b: &Option<T>, t: f32) -> Option<Option<T>> {
    match (a, b) {
        (Some(x), Some(y)) => T::interp(x, y, t).map(Some),
        (None, None) => Some(None),
        _ => None,
    }
}

fn opt_same<T: Interp>(a: &Option<T>, b: &Option<T>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => T::same(x, y),
        (None, None) => true,
        _ => false,
    }
}

/// Colour-valued optional: `None` (no colour) interpolates as transparent.
fn color_opt(a: &Option<TextColor>, b: &Option<TextColor>, t: f32) -> Option<TextColor> {
    if a.is_none() && b.is_none() {
        return None;
    }
    let tr = TextColor::rgba(0, 0, 0, 0);
    TextColor::interp(&a.unwrap_or(tr), &b.unwrap_or(tr), t)
}

fn translate_key(t: Option<(TranslateAxis, TranslateAxis)>) -> Option<(f32, f32, f32, f32)> {
    t.map(|(x, y)| (x.px, x.frac, y.px, y.frac))
}

/// Interpolates every animatable field; every other field is chosen
/// discretely (A below 50 %, B from 50 %).
pub(crate) fn lerp_styles(a: &ComputedStyle, b: &ComputedStyle, t: f32) -> ComputedStyle {
    let mut r = if t < 0.5 { *a } else { *b };
    macro_rules! plain { ($($f:ident),*) => { $(if let Some(v) = f32::interp(&a.$f, &b.$f, t) { r.$f = v; })* }; }
    plain!(opacity, font_size, word_spacing, text_indent, outline_width, flex_grow, flex_shrink);
    r.color = TextColor::interp(&a.color, &b.color, t).unwrap_or(r.color);
    r.background_color = color_opt(&a.background_color, &b.background_color, t);
    r.outline_color = color_opt(&a.outline_color, &b.outline_color, t);
    r.text_decoration_color = color_opt(&a.text_decoration_color, &b.text_decoration_color, t);
    for i in 0..4 {
        r.border_color[i] = color_opt(&a.border_color[i], &b.border_color[i], t);
        if let Some(v) = f32::interp(&a.border_width[i], &b.border_width[i], t) {
            r.border_width[i] = v;
        }
    }
    macro_rules! blen { ($($($f:ident).+),*) => { $(if let Some(v) = BoxLength::interp(&a.$($f).+, &b.$($f).+, t) { r.$($f).+ = v; })* }; }
    if !(a.margin_auto.0 || b.margin_auto.0 || a.margin_auto.1 || b.margin_auto.1) {
        blen!(margin.top, margin.right, margin.bottom, margin.left);
    }
    blen!(padding.top, padding.right, padding.bottom, padding.left, border_radius);
    macro_rules! oblen { ($($($f:ident).+),*) => { $(if let Some(v) = opt_interp(&a.$($f).+, &b.$($f).+, t) { r.$($f).+ = v; })* }; }
    oblen!(width, height, min_width, max_width, min_height, max_height);
    oblen!(inset.top, inset.right, inset.bottom, inset.left, z_index);
    r.translate = match (a.translate, b.translate) {
        (None, None) => None,
        (x, y) => {
            let (xa, ya) = x.unwrap_or((ZERO_AXIS, ZERO_AXIS));
            let (xb, yb) = y.unwrap_or((ZERO_AXIS, ZERO_AXIS));
            Some((axis_lerp(xa, xb, t), axis_lerp(ya, yb, t)))
        }
    };
    if a.visibility != b.visibility && t > 0.0 && t < 1.0 {
        r.visibility = Visibility::Visible;
    }
    r
}

/// Copies onto `r` every interpolable field in which `s` differs from
/// `base`, so a later animation of the list does not undo an earlier one.
fn merge_styles(r: &mut ComputedStyle, s: &ComputedStyle, base: &ComputedStyle) {
    macro_rules! m { ($same:expr, $($f:ident).+) => { if !$same(&s.$($f).+, &base.$($f).+) { r.$($f).+ = s.$($f).+; } }; }
    macro_rules! plain { ($($f:ident),*) => { $(m!(f32::same, $f);)* }; }
    plain!(opacity, font_size, word_spacing, text_indent, outline_width, flex_grow, flex_shrink);
    m!(TextColor::same, color);
    m!(opt_same, background_color);
    m!(opt_same, outline_color);
    m!(opt_same, text_decoration_color);
    for i in 0..4 {
        if !opt_same(&s.border_color[i], &base.border_color[i]) {
            r.border_color[i] = s.border_color[i];
        }
        if s.border_width[i] != base.border_width[i] {
            r.border_width[i] = s.border_width[i];
        }
    }
    m!(BoxLength::same, margin.top);
    m!(BoxLength::same, margin.right);
    m!(BoxLength::same, margin.bottom);
    m!(BoxLength::same, margin.left);
    m!(BoxLength::same, padding.top);
    m!(BoxLength::same, padding.right);
    m!(BoxLength::same, padding.bottom);
    m!(BoxLength::same, padding.left);
    m!(BoxLength::same, border_radius);
    m!(opt_same, width);
    m!(opt_same, height);
    m!(opt_same, min_width);
    m!(opt_same, max_width);
    m!(opt_same, min_height);
    m!(opt_same, max_height);
    m!(opt_same, inset.top);
    m!(opt_same, inset.right);
    m!(opt_same, inset.bottom);
    m!(opt_same, inset.left);
    m!(opt_same, z_index);
    if translate_key(s.translate) != translate_key(base.translate) {
        r.translate = s.translate;
    }
    if s.visibility != base.visibility {
        r.visibility = s.visibility;
    }
}

/// Applies the running animations of `base` (whose lists are `lists`) at
/// the document clock. Called only when an `animation` is declared.
pub(crate) fn evaluate(
    base: ComputedStyle,
    lists: &AnimLonghands,
    handle: &Handle,
    rules: &[StyleRule],
    inherited: InheritedStyle,
    viewport_width: f32,
    pseudo: Option<&str>,
) -> ComputedStyle {
    let now = DOC_CLOCK.with(|c| c.get());
    let order_after_base = STYLE_ORDER.with(|c| c.get());
    let mut result: Option<ComputedStyle> = None;
    for (idx, a) in lists.resolve().into_iter().enumerate() {
        let Some(name) = a.name.as_deref() else { continue };
        let Some(kf) = lookup_keyframes(name, viewport_width) else { continue };
        if kf.is_empty() {
            continue;
        }
        let start = start_time(handle, pseudo.is_some(), idx, name);
        ANIM_REPORT.with(|r| {
            r.borrow_mut().push(AnimRecord {
                key: (node_id(handle), pseudo.map(|p| format!("::{}", p.trim_start_matches(':'))), idx, name.to_string()),
                node: Rc::downgrade(handle),
                anim: a.clone(),
                start,
            })
        });
        let elapsed = if a.play_state == AnimPlayState::Paused { 0.0 } else { now - start };
        let Some(p) = directed_progress(&a, elapsed) else { continue };
        let (lo, from, hi, to) = interval(&kf, p);
        let local = if hi > lo { (p - lo) / (hi - lo) } else { 0.0 };
        let timing = from
            .iter()
            .rev()
            .find_map(|k| k.decls.anim.as_ref().and_then(|d| d.timing.as_ref()).and_then(|t| t.first().copied()))
            .unwrap_or(a.timing);
        let t = timing.ease(local);
        let ds_a: Vec<&Declarations> = from.iter().map(|k| &k.decls).collect();
        let ds_b: Vec<&Declarations> = to.iter().map(|k| &k.decls).collect();
        let sa = if ds_a.is_empty() { base } else { resolve_style_core(handle, rules, inherited, viewport_width, pseudo, &ds_a) };
        let sb = if ds_b.is_empty() { base } else { resolve_style_core(handle, rules, inherited, viewport_width, pseudo, &ds_b) };
        let s = lerp_styles(&sa, &sb, t);
        match result.as_mut() {
            None => result = Some(s),
            Some(r) => merge_styles(r, &s, &base),
        }
    }
    STYLE_ORDER.with(|c| c.set(order_after_base));
    match result {
        None => base,
        Some(mut r) => {
            r.node = base.node;
            r.tree_order = base.tree_order;
            r.animations = base.animations;
            r
        }
    }
}
