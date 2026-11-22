//! QANIMHLb: CSS Transitions 1. Parsing of `transition` and its five longhands (into
//! `Declarations::trans`, cascaded into an interned `TransRef` on `ComputedStyle`, the same scheme as
//! the animation lists, QY53i3), and the transition engine.
//!
//! Engine: `apply` runs at the end of every `resolve_style_for` (only for an element that declares
//! `transition` or has an entry). Each element with transitions keeps its last base style (the
//! after-change style of the previous style change, without transitions); a resolve whose base value
//! of a transitionable property differs from it starts (or reverses / replaces) a transition from the
//! before-change value (the kept base, or the running transition's current value) to the new one.
//! The comparison is idempotent: a second resolve of the same state (a geometry read, the
//! incremental engine's check build) sees no change and only applies the running transitions at the
//! document clock. Read-only resolves (the incremental shadow run) never start or cancel anything.
//! Transitions are applied on top of the base style except where a CSS animation set the value.
//!
//! `tick` (the rendering update's animation step, App and headless) walks the running transitions at
//! the clock, sends `transitionrun`/`transitionstart`/`transitionend`/`transitioncancel` by phase
//! change, drops finished ones, and says whether a restyle frame is due (no more often than the
//! caller's frame interval) and when the next change comes. Nothing running = nothing scheduled.
use super::*;

/// One entry of `transition-property`.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum TransProp {
    All,
    None,
    Named(String),
}

/// The declared lists of one declaration block (`None` = not declared here). Times in ms.
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct TransDecls {
    pub(crate) property: Option<Vec<TransProp>>,
    pub(crate) duration: Option<Vec<f32>>,
    pub(crate) timing: Option<Vec<TimingFunction>>,
    pub(crate) delay: Option<Vec<f32>>,
    /// `transition-behavior`: true = `allow-discrete`.
    pub(crate) behavior: Option<Vec<bool>>,
}

/// The computed lists of one element (empty = the initial value).
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct TransLonghands {
    pub(crate) property: Vec<TransProp>,
    pub(crate) duration: Vec<f32>,
    pub(crate) timing: Vec<TimingFunction>,
    pub(crate) delay: Vec<f32>,
    pub(crate) behavior: Vec<bool>,
}

/// One entry of the zipped list.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Transition {
    pub(crate) property: TransProp,
    pub(crate) duration: f32,
    pub(crate) timing: TimingFunction,
    pub(crate) delay: f32,
    pub(crate) discrete: bool,
}

fn cyc<T: Clone>(v: &[T], i: usize, default: T) -> T {
    if v.is_empty() { default } else { v[i % v.len()].clone() }
}

impl TransLonghands {
    /// CSS Transitions 1 §2: the list is as long as `transition-property`'s (initial `all`); the
    /// other longhands repeat or are truncated to it.
    pub(crate) fn resolve(&self) -> Vec<Transition> {
        let props = if self.property.is_empty() { vec![TransProp::All] } else { self.property.clone() };
        props
            .into_iter()
            .enumerate()
            .map(|(i, property)| Transition {
                property,
                duration: cyc(&self.duration, i, 0.0),
                timing: cyc(&self.timing, i, TimingFunction::EASE),
                delay: cyc(&self.delay, i, 0.0),
                discrete: cyc(&self.behavior, i, false),
            })
            .collect()
    }
}

/// A `Copy` handle to one interned `TransLonghands` (as `AnimRef`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct TransRef {
    generation: std::num::NonZeroU32,
    index: u32,
}

impl TransRef {
    pub(crate) fn table(self) -> Option<Rc<TransLonghands>> {
        TRANS_ARENA.with(|a| a.borrow().get(self).cloned())
    }
}

struct TransArena {
    generation: std::num::NonZeroU32,
    tables: Vec<Rc<TransLonghands>>,
    index: HashMap<String, u32>,
}

static TRANS_GENERATION: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

impl TransArena {
    fn new() -> Self {
        let g = TRANS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let generation = std::num::NonZeroU32::new(g).unwrap_or(std::num::NonZeroU32::MIN);
        TransArena { generation, tables: Vec::new(), index: HashMap::new() }
    }

    fn get(&self, r: TransRef) -> Option<&Rc<TransLonghands>> {
        if r.generation != self.generation {
            return None;
        }
        self.tables.get(r.index as usize)
    }
}

thread_local! {
    static TRANS_ARENA: RefCell<TransArena> = RefCell::new(TransArena::new());
    /// A `transition` declaration was parsed on this thread since the last reset (the headless style
    /// step runs only for pages that declare transitions or `@keyframes`).
    static DECLARED: Cell<bool> = const { Cell::new(false) };
}

/// Frees the interned lists (navigation; called from `reset_anim_arena`).
pub(crate) fn reset_trans_arena() {
    let old = TRANS_ARENA.with(|a| std::mem::replace(&mut *a.borrow_mut(), TransArena::new()));
    drop(old);
}

/// Whether some `transition` declaration was parsed on this thread (never reset: parsed rules are
/// cached across navigations).
pub(crate) fn any_declared() -> bool {
    DECLARED.with(Cell::get)
}

/// A `transition` declaration was written (an inline `style`, before it is parsed).
pub(crate) fn note_declared() {
    DECLARED.with(|d| d.set(true));
}

pub(crate) fn intern_trans(a: TransLonghands) -> Option<TransRef> {
    if a == TransLonghands::default() {
        return None;
    }
    TRANS_ARENA.with(|ar| {
        let mut ar = ar.borrow_mut();
        let generation = ar.generation;
        let key = format!("{a:?}");
        if let Some(&index) = ar.index.get(&key) {
            return Some(TransRef { generation, index });
        }
        let index = u32::try_from(ar.tables.len()).expect("fewer than 2^32 transition list sets");
        ar.tables.push(Rc::new(a));
        ar.index.insert(key, index);
        Some(TransRef { generation, index })
    })
}

pub(crate) const TRANSITION_KEYS: [&str; 5] =
    ["transition-property", "transition-duration", "transition-timing-function", "transition-delay", "transition-behavior"];

pub(crate) fn is_transition_property(name: &str) -> bool {
    name.eq_ignore_ascii_case("transition") || TRANSITION_KEYS.iter().any(|k| name.eq_ignore_ascii_case(k))
}

type PErr = ParseError<()>;

fn parse_prop<'i>(i: &mut CssParser<'i>) -> Result<TransProp, PErr> {
    match i.next()?.clone() {
        Token::Ident(id) if id.eq_ignore_ascii_case("none") => Ok(TransProp::None),
        Token::Ident(id) if id.eq_ignore_ascii_case("all") => Ok(TransProp::All),
        Token::Ident(id) if !is_reserved_name(&id) => Ok(TransProp::Named(id.to_ascii_lowercase())),
        _ => Err(i.new_error_for_next_token()),
    }
}

fn parse_nonneg_time<'i>(i: &mut CssParser<'i>) -> Result<f32, PErr> {
    let t = i.next()?.clone();
    match parse_time(&t) {
        Some(v) if v >= 0.0 => Ok(v),
        _ => Err(i.new_error_for_next_token()),
    }
}

fn parse_any_time<'i>(i: &mut CssParser<'i>) -> Result<f32, PErr> {
    let t = i.next()?.clone();
    parse_time(&t).ok_or_else(|| i.new_error_for_next_token())
}

fn parse_behavior<'i>(i: &mut CssParser<'i>) -> Result<bool, PErr> {
    let id = i.expect_ident()?.clone();
    if id.eq_ignore_ascii_case("allow-discrete") {
        Ok(true)
    } else if id.eq_ignore_ascii_case("normal") {
        Ok(false)
    } else {
        Err(i.new_error_for_next_token())
    }
}

/// One `<single-transition>`: `[none | <property>] || <time> || <easing> || <time> || <behavior>`.
fn parse_single<'i>(i: &mut CssParser<'i>) -> Result<Transition, PErr> {
    let (mut prop, mut dur, mut delay, mut timing, mut beh) = (None, None, None, None, None);
    let mut any = false;
    while !i.is_exhausted() {
        any = true;
        let st = i.state();
        let tok = i.next()?.clone();
        match tok {
            Token::Dimension { .. } => {
                let Some(t) = parse_time(&tok) else { return Err(i.new_error_for_next_token()) };
                if dur.is_none() {
                    if t < 0.0 {
                        return Err(i.new_error_for_next_token());
                    }
                    dur = Some(t);
                } else if delay.is_none() {
                    delay = Some(t);
                } else {
                    return Err(i.new_error_for_next_token());
                }
            }
            Token::Function(_) if timing.is_none() => {
                i.reset(&st);
                timing = Some(parse_timing(i)?);
            }
            Token::Ident(id) => {
                let kw = timing_keyword(&id);
                if let (Some(t), true) = (kw, timing.is_none()) {
                    timing = Some(t);
                } else if beh.is_none() && (id.eq_ignore_ascii_case("allow-discrete") || id.eq_ignore_ascii_case("normal")) {
                    beh = Some(id.eq_ignore_ascii_case("allow-discrete"));
                } else if prop.is_none() {
                    i.reset(&st);
                    prop = Some(parse_prop(i)?);
                } else {
                    return Err(i.new_error_for_next_token());
                }
            }
            _ => return Err(i.new_error_for_next_token()),
        }
    }
    if !any {
        return Err(i.new_error_for_next_token());
    }
    Ok(Transition {
        property: prop.unwrap_or(TransProp::All),
        duration: dur.unwrap_or(0.0),
        timing: timing.unwrap_or(TimingFunction::EASE),
        delay: delay.unwrap_or(0.0),
        discrete: beh.unwrap_or(false),
    })
}

fn list<'i, T>(i: &mut CssParser<'i>, f: impl FnMut(&mut CssParser<'i>) -> Result<T, PErr>) -> Result<Vec<T>, PErr> {
    let v = i.parse_comma_separated(f)?;
    i.expect_exhausted()?;
    Ok(v)
}

/// Parses the (already `!important`-stripped) value of `transition` or one longhand (unprefixed).
/// `none` is only valid as the whole `transition-property` / shorthand list.
pub(crate) fn parse_transition_value(name: &str, value: &str) -> Result<TransDecls, ()> {
    let mut input = CssParser::new(value);
    let i = &mut input;
    let mut d = TransDecls::default();
    let only_none = |v: &[TransProp]| v.len() == 1 || !v.contains(&TransProp::None);
    let r: Result<(), PErr> = (|| {
        match name.to_ascii_lowercase().as_str() {
            "transition" => {
                let v = list(i, parse_single)?;
                let props: Vec<TransProp> = v.iter().map(|t| t.property.clone()).collect();
                if !only_none(&props) {
                    return Err(i.new_error_for_next_token());
                }
                d.property = Some(props);
                d.duration = Some(v.iter().map(|t| t.duration).collect());
                d.timing = Some(v.iter().map(|t| t.timing).collect());
                d.delay = Some(v.iter().map(|t| t.delay).collect());
                d.behavior = Some(v.iter().map(|t| t.discrete).collect());
            }
            "transition-property" => {
                let v = list(i, parse_prop)?;
                if !only_none(&v) {
                    return Err(i.new_error_for_next_token());
                }
                d.property = Some(v);
            }
            "transition-duration" => d.duration = Some(list(i, parse_nonneg_time)?),
            "transition-timing-function" => d.timing = Some(list(i, parse_timing)?),
            "transition-delay" => d.delay = Some(list(i, parse_any_time)?),
            "transition-behavior" => d.behavior = Some(list(i, parse_behavior)?),
            _ => return Err(i.new_error_for_next_token()),
        }
        Ok(())
    })();
    r.map(|_| d).map_err(|_| ())
}

impl Declarations {
    pub(crate) fn set_transition(&mut self, d: TransDecls) {
        let a = self.trans.get_or_insert_with(Default::default);
        macro_rules! m { ($($f:ident),*) => { $(if d.$f.is_some() { a.$f = d.$f; })* }; }
        m!(property, duration, timing, delay, behavior);
    }
}

impl DeclParser<'_> {
    /// `transition` and its longhands (already un-prefixed by the caller).
    pub(crate) fn apply_transition(&mut self, name: &str, input: &mut CssParser<'_>) {
        let start = input.state();
        while input.next().is_ok() {}
        let raw = input.slice_from(start.position()).trim();
        let raw = strip_important(raw).unwrap_or(raw);
        if let Ok(d) = parse_transition_value(name, raw) {
            DECLARED.with(|f| f.set(true));
            self.decls.set_transition(d);
        }
    }
}

/// The per-element cascade of the five longhands (a CSS-wide keyword winner = the initial value).
pub(crate) fn cascade_transitions(matches: &[(u64, &Declarations)], inline: Option<&Declarations>) -> Option<TransRef> {
    if !(matches.iter().any(|(_, d)| d.trans.is_some()) || inline.is_some_and(|d| d.trans.is_some())) {
        return None;
    }
    macro_rules! pick {
        ($key:literal, $f:ident) => {
            match cascade_css(matches, inline, $key, |d| d.trans.as_ref().and_then(|a| a.$f.clone())) {
                Cascaded::Value(v) => v,
                _ => Vec::new(),
            }
        };
    }
    intern_trans(TransLonghands {
        property: pick!("transition-property", property),
        duration: pick!("transition-duration", duration),
        timing: pick!("transition-timing-function", timing),
        delay: pick!("transition-delay", delay),
        behavior: pick!("transition-behavior", behavior),
    })
}

// ---------------------------------------------------------------------------------------------
// The transitionable properties (the ones the animation engine interpolates, `lerp_styles`).

type Same = fn(&ComputedStyle, &ComputedStyle) -> bool;
type Lerp = fn(&mut ComputedStyle, &ComputedStyle, &ComputedStyle, f32) -> bool;
type Copy1 = fn(&mut ComputedStyle, &ComputedStyle);

pub(crate) struct PropDef {
    pub(crate) name: &'static str,
    inherited: bool,
    same: Same,
    /// Writes the interpolated value into the first argument; false = the pair does not interpolate.
    lerp: Lerp,
    copy: Copy1,
}

fn same_f32(a: &f32, b: &f32) -> bool {
    a == b
}
fn lerp_f32(a: &f32, b: &f32, t: f32) -> Option<f32> {
    Some(lerp(*a, *b, t))
}
fn same_color(a: &TextColor, b: &TextColor) -> bool {
    a == b
}
fn lerp_color(a: &TextColor, b: &TextColor, t: f32) -> Option<TextColor> {
    TextColor::interp(a, b, t)
}
fn same_ocolor(a: &Option<TextColor>, b: &Option<TextColor>) -> bool {
    opt_same(a, b)
}
fn lerp_ocolor(a: &Option<TextColor>, b: &Option<TextColor>, t: f32) -> Option<Option<TextColor>> {
    Some(color_opt(a, b, t))
}
fn same_blen(a: &BoxLength, b: &BoxLength) -> bool {
    BoxLength::same(a, b)
}
fn lerp_blen(a: &BoxLength, b: &BoxLength, t: f32) -> Option<BoxLength> {
    BoxLength::interp(a, b, t)
}
fn same_oblen(a: &Option<BoxLength>, b: &Option<BoxLength>) -> bool {
    opt_same(a, b)
}
fn lerp_oblen(a: &Option<BoxLength>, b: &Option<BoxLength>, t: f32) -> Option<Option<BoxLength>> {
    match (a, b) {
        (None, None) => Some(None),
        _ => opt_interp(a, b, t),
    }
}
fn same_oi32(a: &Option<i32>, b: &Option<i32>) -> bool {
    a == b
}
fn lerp_oi32(a: &Option<i32>, b: &Option<i32>, t: f32) -> Option<Option<i32>> {
    opt_interp(a, b, t)
}
type Tr = Option<(TranslateAxis, TranslateAxis)>;
fn tkey(t: &Tr) -> Option<(f32, f32, f32, f32)> {
    t.map(|(x, y)| (x.px, x.frac, y.px, y.frac))
}
fn same_tr(a: &Tr, b: &Tr) -> bool {
    tkey(a) == tkey(b)
}
fn lerp_tr(a: &Tr, b: &Tr, t: f32) -> Option<Tr> {
    if a.is_none() && b.is_none() {
        return Some(None);
    }
    let (xa, ya) = a.unwrap_or((ZERO_AXIS, ZERO_AXIS));
    let (xb, yb) = b.unwrap_or((ZERO_AXIS, ZERO_AXIS));
    Some(Some((axis_lerp(xa, xb, t), axis_lerp(ya, yb, t))))
}
fn same_vis(a: &Visibility, b: &Visibility) -> bool {
    a == b
}
/// `visibility` interpolates as visible strictly between its ends when one end is `visible`.
fn lerp_vis(a: &Visibility, b: &Visibility, t: f32) -> Option<Visibility> {
    if a == b {
        return Some(*a);
    }
    if *a != Visibility::Visible && *b != Visibility::Visible {
        return None;
    }
    Some(if t <= 0.0 {
        *a
    } else if t >= 1.0 {
        *b
    } else {
        Visibility::Visible
    })
}

macro_rules! pdef {
    ($name:literal, $inh:expr, $kind:ident, $($f:tt)+) => {
        paste_kind!($kind, $name, $inh, $($f)+)
    };
}

macro_rules! paste_kind {
    (f32, $name:literal, $inh:expr, $($f:tt)+) => { pd!($name, $inh, same_f32, lerp_f32, $($f)+) };
    (color, $name:literal, $inh:expr, $($f:tt)+) => { pd!($name, $inh, same_color, lerp_color, $($f)+) };
    (ocolor, $name:literal, $inh:expr, $($f:tt)+) => { pd!($name, $inh, same_ocolor, lerp_ocolor, $($f)+) };
    (blen, $name:literal, $inh:expr, $($f:tt)+) => { pd!($name, $inh, same_blen, lerp_blen, $($f)+) };
    (oblen, $name:literal, $inh:expr, $($f:tt)+) => { pd!($name, $inh, same_oblen, lerp_oblen, $($f)+) };
    (oi32, $name:literal, $inh:expr, $($f:tt)+) => { pd!($name, $inh, same_oi32, lerp_oi32, $($f)+) };
    (tr, $name:literal, $inh:expr, $($f:tt)+) => { pd!($name, $inh, same_tr, lerp_tr, $($f)+) };
    (vis, $name:literal, $inh:expr, $($f:tt)+) => { pd!($name, $inh, same_vis, lerp_vis, $($f)+) };
}

macro_rules! pd {
    ($name:literal, $inh:expr, $same:ident, $lerp:ident, $($f:tt)+) => {
        PropDef {
            name: $name,
            inherited: $inh,
            same: |a, b| $same(&a.$($f)+, &b.$($f)+),
            lerp: |r, a, b, t| match $lerp(&a.$($f)+, &b.$($f)+, t) {
                Some(v) => {
                    r.$($f)+ = v;
                    true
                }
                None => false,
            },
            copy: |r, a| r.$($f)+ = a.$($f)+,
        }
    };
}

/// Margins with an `auto` end do not interpolate.
fn margin_def(name: &'static str, side: usize) -> PropDef {
    fn auto(s: &ComputedStyle, side: usize) -> bool {
        match side {
            0 => s.margin_auto_v.0,
            1 => s.margin_auto.1,
            2 => s.margin_auto_v.1,
            _ => s.margin_auto.0,
        }
    }
    fn get(s: &mut ComputedStyle, side: usize) -> &mut BoxLength {
        match side {
            0 => &mut s.margin.top,
            1 => &mut s.margin.right,
            2 => &mut s.margin.bottom,
            _ => &mut s.margin.left,
        }
    }
    macro_rules! side_def {
        ($n:expr, $i:literal) => {
            PropDef {
                name: $n,
                inherited: false,
                same: |a, b| auto(a, $i) == auto(b, $i) && BoxLength::same(get(&mut a.clone(), $i), get(&mut b.clone(), $i)),
                lerp: |r, a, b, t| {
                    if auto(a, $i) || auto(b, $i) {
                        return false;
                    }
                    match BoxLength::interp(get(&mut a.clone(), $i), get(&mut b.clone(), $i), t) {
                        Some(v) => {
                            *get(r, $i) = v;
                            true
                        }
                        None => false,
                    }
                },
                copy: |r, a| {
                    *get(r, $i) = *get(&mut a.clone(), $i);
                    match $i {
                        0 => r.margin_auto_v.0 = a.margin_auto_v.0,
                        1 => r.margin_auto.1 = a.margin_auto.1,
                        2 => r.margin_auto_v.1 = a.margin_auto_v.1,
                        _ => r.margin_auto.0 = a.margin_auto.0,
                    }
                },
            }
        };
    }
    match side {
        0 => side_def!(name, 0),
        1 => side_def!(name, 1),
        2 => side_def!(name, 2),
        _ => side_def!(name, 3),
    }
}

thread_local! {
    static PROPS: Vec<PropDef> = build_props();
}

fn build_props() -> Vec<PropDef> {
    vec![
        pdef!("opacity", false, f32, opacity),
        pdef!("color", true, color, color),
        pdef!("background-color", false, ocolor, background_color),
        pdef!("outline-color", false, ocolor, outline_color),
        pdef!("outline-width", false, f32, outline_width),
        pdef!("text-decoration-color", false, ocolor, text_decoration_color),
        pdef!("border-top-color", false, ocolor, border_color[0]),
        pdef!("border-right-color", false, ocolor, border_color[1]),
        pdef!("border-bottom-color", false, ocolor, border_color[2]),
        pdef!("border-left-color", false, ocolor, border_color[3]),
        pdef!("border-top-width", false, f32, border_width[0]),
        pdef!("border-right-width", false, f32, border_width[1]),
        pdef!("border-bottom-width", false, f32, border_width[2]),
        pdef!("border-left-width", false, f32, border_width[3]),
        margin_def("margin-top", 0),
        margin_def("margin-right", 1),
        margin_def("margin-bottom", 2),
        margin_def("margin-left", 3),
        pdef!("padding-top", false, blen, padding.top),
        pdef!("padding-right", false, blen, padding.right),
        pdef!("padding-bottom", false, blen, padding.bottom),
        pdef!("padding-left", false, blen, padding.left),
        pdef!("border-radius", false, blen, border_radius),
        pdef!("width", false, oblen, width),
        pdef!("height", false, oblen, height),
        pdef!("min-width", false, oblen, min_width),
        pdef!("max-width", false, oblen, max_width),
        pdef!("min-height", false, oblen, min_height),
        pdef!("max-height", false, oblen, max_height),
        pdef!("top", false, oblen, inset.top),
        pdef!("right", false, oblen, inset.right),
        pdef!("bottom", false, oblen, inset.bottom),
        pdef!("left", false, oblen, inset.left),
        pdef!("z-index", false, oi32, z_index),
        pdef!("font-size", true, f32, font_size),
        pdef!("word-spacing", true, f32, word_spacing),
        pdef!("text-indent", true, f32, text_indent),
        pdef!("flex-grow", false, f32, flex_grow),
        pdef!("flex-shrink", false, f32, flex_shrink),
        pdef!("transform", false, tr, translate),
        pdef!("visibility", true, vis, visibility),
    ]
}

/// Whether the `transition-property` entry `p` covers the longhand `name` (shorthands expand).
fn covers(p: &TransProp, name: &str) -> bool {
    let n = match p {
        TransProp::All => return true,
        TransProp::None => return false,
        TransProp::Named(n) => n.as_str(),
    };
    if n == name {
        return true;
    }
    let side = |pre: &str, post: &str| name.strip_prefix(pre).and_then(|r| r.strip_suffix(post)).is_some_and(|m| ["top", "right", "bottom", "left"].contains(&m));
    match n {
        "translate" => name == "transform",
        "border-color" => side("border-", "-color"),
        "border-width" => side("border-", "-width"),
        "border" => side("border-", "-color") || side("border-", "-width"),
        "border-top" | "border-right" | "border-bottom" | "border-left" => name.starts_with(n) && name.len() > n.len(),
        "margin" => name.starts_with("margin-"),
        "padding" => name.starts_with("padding-"),
        "inset" => ["top", "right", "bottom", "left"].contains(&name),
        "outline" => name == "outline-color" || name == "outline-width",
        "background" => name == "background-color",
        "text-decoration" => name == "text-decoration-color",
        "flex" => name == "flex-grow" || name == "flex-shrink",
        "font" => name == "font-size",
        _ => false,
    }
}

/// The transition list entry for longhand `name`: the last matching entry wins (§2.1).
fn matching<'a>(list: &'a [Transition], name: &str) -> Option<&'a Transition> {
    list.iter().rev().find(|t| covers(&t.property, name))
}

// ---------------------------------------------------------------------------------------------
// The engine.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// No event sent yet (the transition is pending: `transitionrun` goes out at the next tick).
    Idle,
    Before,
    Active,
    After,
}

struct Running {
    prop: usize,
    from: ComputedStyle,
    to: ComputedStyle,
    /// The reversing-adjusted start value (§3.1).
    rev_start: ComputedStyle,
    shortening: f64,
    start: f64,
    delay: f64,
    duration: f64,
    timing: TimingFunction,
    discrete: bool,
    /// The phase the last tick reported.
    phase: Phase,
    /// `transitionend` was sent; dropped at the following tick (an inheriting child's restyle in
    /// between still sees that its parent's value was a transition's, not a new style).
    ended: bool,
}

impl Running {
    fn phase_at(&self, now: f64) -> Phase {
        let t = now - self.start;
        if t < self.delay {
            Phase::Before
        } else if t < self.delay + self.duration {
            Phase::Active
        } else {
            Phase::After
        }
    }

    /// Output progress at `now` (backwards fill in the delay).
    fn progress(&self, now: f64) -> f32 {
        let t = now - self.start - self.delay;
        if t <= 0.0 || self.duration <= 0.0 {
            return if t >= self.duration && self.duration <= 0.0 && t >= 0.0 { 1.0 } else { 0.0 };
        }
        let x = (t / self.duration).min(1.0) as f32;
        self.timing.ease(x)
    }

    /// Writes this transition's value at `now` into `out`.
    fn write(&self, def: &PropDef, out: &mut ComputedStyle, now: f64) {
        let t = self.progress(now);
        if self.discrete {
            (def.copy)(out, if t < 0.5 { &self.from } else { &self.to });
        } else if !(def.lerp)(out, &self.from, &self.to, t) {
            (def.copy)(out, if t < 0.5 { &self.from } else { &self.to });
        }
    }

    fn active_time(&self, now: f64) -> f64 {
        (now - self.start - self.delay).clamp(0.0, self.duration.max(0.0))
    }
}

macro_rules! snap_fields {
    ($($f:ident: $t:ty),* $(,)?) => {
        /// The transitionable fields of a base style (the before-change style an entry keeps; ~1/5
        /// of a `ComputedStyle`).
        #[derive(Clone, Copy, PartialEq)]
        struct Snap { $($f: $t,)* }
        impl Snap {
            fn of(s: &ComputedStyle) -> Snap {
                Snap { $($f: s.$f,)* }
            }
            /// `base` with these fields: the before-change style for the comparisons.
            fn over(&self, base: &ComputedStyle) -> ComputedStyle {
                let mut r = *base;
                $(r.$f = self.$f;)*
                r
            }
        }
    };
}

snap_fields! {
    opacity: f32, color: TextColor, background_color: Option<TextColor>, outline_color: Option<TextColor>,
    outline_width: f32, text_decoration_color: Option<TextColor>, border_color: [Option<TextColor>; 4],
    border_width: [f32; 4], margin: Sides, margin_auto: (bool, bool), margin_auto_v: (bool, bool),
    padding: Sides, border_radius: BoxLength, width: Option<BoxLength>, height: Option<BoxLength>,
    min_width: Option<BoxLength>, max_width: Option<BoxLength>, min_height: Option<BoxLength>,
    max_height: Option<BoxLength>, inset: Insets, z_index: Option<i32>, font_size: f32, word_spacing: f32,
    text_indent: f32, flex_grow: f32, flex_shrink: f32, translate: Tr, visibility: Visibility,
}

struct ElemTrans {
    node: std::rc::Weak<markup5ever_rcdom::Node>,
    pseudo: String,
    /// The base (untransitioned) style the last resolve computed: the next resolve's before-change style.
    prev: Snap,
    running: Vec<Running>,
    /// The element declares `transition`, or a getComputedStyle read wants its before-change style
    /// kept (else the entry goes once nothing runs).
    declares: bool,
    seen: u64,
}

/// A transition event to dispatch at `node` (already gone = `None`).
pub(crate) struct TransEvent {
    pub(crate) kind: &'static str,
    pub(crate) node: Option<Handle>,
    pub(crate) property: &'static str,
    pub(crate) elapsed_s: f64,
    pub(crate) pseudo: String,
}

#[derive(Default)]
pub(crate) struct TransTick {
    pub(crate) events: Vec<TransEvent>,
    /// A restyle is due (a running transition's value moved, or one ended).
    pub(crate) restyle: bool,
    /// Clock (ms) of the next change, `None` when nothing is scheduled.
    pub(crate) next: Option<f64>,
}

type Key = (usize, u8);

thread_local! {
    static STORE: RefCell<HashMap<Key, ElemTrans>> = RefCell::new(HashMap::new());
    /// Entries exist (so an element without `transition` pays one flag test).
    static ANY: Cell<bool> = const { Cell::new(false) };
    /// Running (not yet ended) transitions, all elements.
    static RUNNING: Cell<usize> = const { Cell::new(0) };
    /// Events owed by transitions cancelled at a style change (sent by the next tick).
    static PENDING: RefCell<Vec<TransEvent>> = const { RefCell::new(Vec::new()) };
    /// Set while getComputedStyle resolves (`with_watch`).
    static WATCH: Cell<bool> = const { Cell::new(false) };
    /// Clock of the last in-between restyle frame `tick` asked for.
    static LAST_FRAME: Cell<f64> = const { Cell::new(f64::NEG_INFINITY) };
}

/// Runs `f` (a getComputedStyle resolve) with before-change styles kept for every element it resolves.
pub(crate) fn with_watch<R>(f: impl FnOnce() -> R) -> R {
    let old = WATCH.with(|w| w.replace(true));
    let r = f();
    WATCH.with(|w| w.set(old));
    r
}

/// Forgets every transition (navigation).
pub(crate) fn reset() {
    STORE.with(|s| s.borrow_mut().clear());
    PENDING.with(|p| p.borrow_mut().clear());
    ANY.with(|a| a.set(false));
    RUNNING.with(|r| r.set(0));
    LAST_FRAME.with(|f| f.set(f64::NEG_INFINITY));
}

fn recount(s: &HashMap<Key, ElemTrans>) {
    ANY.with(|a| a.set(!s.is_empty()));
    RUNNING.with(|r| r.set(s.values().map(|e| e.running.iter().filter(|r| !r.ended).count()).sum()));
}

/// Something is running or an event is owed: the caller keeps ticking.
pub(crate) fn live() -> bool {
    RUNNING.with(Cell::get) > 0
        || PENDING.with(|p| !p.borrow().is_empty())
        || (ANY.with(Cell::get) && STORE.with(|s| s.borrow().values().any(|e| !e.running.is_empty())))
}

/// A transition was started or cancelled since the last tick (its first event is owed).
pub(crate) fn owes_events() -> bool {
    PENDING.with(|p| !p.borrow().is_empty())
        || (RUNNING.with(Cell::get) > 0 && STORE.with(|s| s.borrow().values().any(|e| e.running.iter().any(|r| r.phase == Phase::Idle))))
}

/// Whether `(node, pseudo slot)` has a running (not ended) transition: its value follows the clock.
pub(crate) fn has_running(node: usize, pseudo: u8) -> bool {
    if RUNNING.with(Cell::get) == 0 {
        return false;
    }
    STORE.with(|s| s.borrow().get(&(node, pseudo)).is_some_and(|e| e.running.iter().any(|r| !r.ended)))
}

fn cancel_event(e: &ElemTrans, r: &Running, now: f64, prop: &'static str) -> Option<TransEvent> {
    (r.phase != Phase::Idle && !r.ended).then(|| TransEvent {
        kind: "transitioncancel",
        node: e.node.upgrade(),
        property: prop,
        elapsed_s: r.active_time(now) / 1000.0,
        pseudo: e.pseudo.clone(),
    })
}

fn ancestor_transitioning(s: &HashMap<Key, ElemTrans>, handle: &Handle, prop: usize, pseudo: bool) -> bool {
    // A pseudo-element inherits from its own element first.
    let mut n = if pseudo { Some(handle.clone()) } else { parent_handle(handle) };
    while let Some(h) = n {
        if !matches!(h.data, NodeData::Element { .. }) {
            break;
        }
        if s.get(&(node_id(&h), 0)).is_some_and(|e| e.running.iter().any(|r| r.prop == prop)) {
            return true;
        }
        n = parent_handle(&h);
    }
    false
}

/// The transition step of `resolve_style_for`: compares `base` (this resolve's untransitioned style)
/// with the element's last one, starts / reverses / cancels transitions, and returns `out` (the base
/// with animations) with the running transitions' values applied.
pub(crate) fn apply(handle: &Handle, pseudo: Option<&str>, base: &ComputedStyle, out: ComputedStyle) -> ComputedStyle {
    if base.transitions.is_none() && !ANY.with(Cell::get) && !WATCH.with(Cell::get) {
        return out;
    }
    let lists: Option<Vec<Transition>> = base.transitions.and_then(|r| r.table()).map(|t| t.resolve());
    let key = (base.node, base.pseudo);
    let now = document_clock_ms();
    let start = crate::anim_eval::start_clock_ms();
    let readonly = crate::anim_eval::readonly();
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        if readonly {
            let Some(e) = s.get(&key) else { return out };
            return PROPS.with(|props| write_running(e, props, base, out, now));
        }
        if base.display == Display::None {
            if let Some(e) = s.remove(&key) {
                PROPS.with(|props| {
                    PENDING.with(|p| p.borrow_mut().extend(e.running.iter().filter_map(|r| cancel_event(&e, r, now, props[r.prop].name))))
                });
                recount(&s);
            }
            return out;
        }
        let pass = crate::anim_eval::current_pass();
        // Inherited properties an ancestor is transitioning: the element's value moved because the
        // ancestor's did (its animation, not a style change of this element), so no transition starts.
        let anc: Vec<bool> = match s.get(&key) {
            Some(e) if RUNNING.with(Cell::get) > 0 && e.prev != Snap::of(base) => PROPS.with(|props| {
                props.iter().enumerate().map(|(i, d)| d.inherited && ancestor_transitioning(&s, handle, i, pseudo.is_some())).collect()
            }),
            _ => Vec::new(),
        };
        let Some(e) = s.get_mut(&key) else {
            // A getComputedStyle read keeps the before-change style of any element, so a script that
            // flushes style, then sets a value and a `transition` together, gets the transition.
            if lists.is_some() || WATCH.with(Cell::get) {
                s.insert(
                    key,
                    ElemTrans {
                        node: Rc::downgrade(handle),
                        pseudo: pseudo.map(|p| format!("::{}", p.trim_start_matches(':'))).unwrap_or_default(),
                        prev: Snap::of(base),
                        running: Vec::new(),
                        declares: true,
                        seen: pass,
                    },
                );
                ANY.with(|a| a.set(true));
            }
            return out;
        };
        e.seen = pass;
        e.declares |= lists.is_some() || WATCH.with(Cell::get);
        let list = lists.as_deref().unwrap_or(&[]);
        let mut cancelled = Vec::new();
        let result = PROPS.with(|props| {
            // A running transition whose property is no longer transitioned is cancelled (§3, step 4).
            let mut i = 0;
            while i < e.running.len() {
                let r = &e.running[i];
                if !r.ended && matching(list, props[r.prop].name).is_none_or(|m| m.duration.max(0.0) + m.delay <= 0.0) {
                    let r = e.running.remove(i);
                    cancelled.extend(cancel_event(e, &r, now, props[r.prop].name));
                } else {
                    i += 1;
                }
            }
            let snap = Snap::of(base);
            if e.prev != snap {
                let before = e.prev.over(base);
                for (pi, def) in props.iter().enumerate() {
                    if (def.same)(&before, base) {
                        continue;
                    }
                    let m = matching(list, def.name).filter(|m| m.duration.max(0.0) + m.delay > 0.0);
                    let pos = e.running.iter().position(|r| r.prop == pi && !r.ended);
                    if anc.get(pi).copied().unwrap_or(false) {
                        continue;
                    }
                    if let Some(p) = pos {
                        if (def.same)(&e.running[p].to, base) {
                            continue;
                        }
                        let old = e.running.remove(p);
                        cancelled.extend(cancel_event(e, &old, now, def.name));
                        let Some(m) = m else { continue };
                        let mut cur = *base;
                        old.write(def, &mut cur, now);
                        if !m.discrete && !(def.lerp)(&mut base.clone(), &cur, base, 0.5) {
                            continue;
                        }
                        let reversing = (def.same)(&old.rev_start, base);
                        let mut r = new_running(pi, cur, *base, cur, m, start);
                        if reversing {
                            // §3.1: the reversed transition is shortened by how far the old one got.
                            let f = ((old.progress(now) as f64) * old.shortening + (1.0 - old.shortening)).abs().clamp(0.0, 1.0);
                            r.shortening = f;
                            r.rev_start = old.to;
                            r.duration *= f;
                            if r.delay < 0.0 {
                                r.delay *= f;
                            }
                        }
                        e.running.push(r);
                    } else if let Some(m) = m {
                        let prev = before;
                        if !m.discrete && !(def.lerp)(&mut base.clone(), &prev, base, 0.5) {
                            continue;
                        }
                        e.running.push(new_running(pi, prev, *base, prev, m, start));
                    }
                }
            }
            e.prev = snap;
            write_running(e, props, base, out, now)
        });
        PENDING.with(|p| p.borrow_mut().extend(cancelled));
        if let Some(e) = s.get(&key)
            && !e.declares
            && e.running.is_empty()
        {
            s.remove(&key);
        }
        recount(&s);
        result
    })
}

fn new_running(prop: usize, from: ComputedStyle, to: ComputedStyle, rev_start: ComputedStyle, m: &Transition, now: f64) -> Running {
    Running {
        prop,
        from,
        to,
        rev_start,
        shortening: 1.0,
        start: now,
        delay: m.delay as f64,
        duration: m.duration.max(0.0) as f64,
        timing: m.timing,
        discrete: m.discrete,
        phase: Phase::Idle,
        ended: false,
    }
}

/// `out` with the element's running transitions applied (where no animation set the value).
fn write_running(e: &ElemTrans, props: &[PropDef], base: &ComputedStyle, mut out: ComputedStyle, now: f64) -> ComputedStyle {
    for r in &e.running {
        if r.ended || r.phase_at(now) == Phase::After {
            continue;
        }
        let def = &props[r.prop];
        if !(def.same)(&out, base) {
            continue;
        }
        r.write(def, &mut out, now);
    }
    out
}

/// End of a from-scratch style pass `pass`: an element not styled in it (removed, or inside a
/// `display: none` subtree) loses its transitions (cancelled).
pub(crate) fn finish_pass(pass: u64) {
    if !ANY.with(Cell::get) {
        return;
    }
    retain_by(|_, e| e.seen == pass && e.node.strong_count() > 0);
}

/// Incremental engine (membership ageing, as `retain_starts`): keeps the entries of the elements
/// `keep` lists.
pub(crate) fn retain(keep: impl Fn(usize) -> bool) {
    if !ANY.with(Cell::get) {
        return;
    }
    retain_by(|k, e| e.node.strong_count() > 0 && keep(k.0));
}

fn retain_by(keep: impl Fn(&Key, &ElemTrans) -> bool) {
    let now = document_clock_ms();
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        let mut ev = Vec::new();
        PROPS.with(|props| {
            s.retain(|k, e| {
                let k2 = keep(k, e);
                if !k2 {
                    ev.extend(e.running.iter().filter_map(|r| cancel_event(e, r, now, props[r.prop].name)));
                }
                k2
            })
        });
        PENDING.with(|p| p.borrow_mut().extend(ev));
        recount(&s);
    });
}

fn ev(e: &ElemTrans, kind: &'static str, prop: &'static str, elapsed_ms: f64) -> TransEvent {
    TransEvent { kind, node: e.node.upgrade(), property: prop, elapsed_s: elapsed_ms / 1000.0, pseudo: e.pseudo.clone() }
}

/// The animation step: events due at clock `now`, whether a restyle is due (in-between frames no
/// more often than every `interval` ms; `f64::INFINITY` = only when a transition ends), and the
/// clock of the next change.
pub(crate) fn tick(now: f64, interval: f64) -> TransTick {
    let mut out = TransTick { events: PENDING.with(|p| std::mem::take(&mut *p.borrow_mut())), ..Default::default() };
    if !ANY.with(Cell::get) {
        return out;
    }
    let mut next: Option<f64> = None;
    let soon = |t: f64, next: &mut Option<f64>| *next = Some(next.map_or(t, |n: f64| n.min(t)));
    let mut active = false;
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        PROPS.with(|props| {
            for e in s.values_mut() {
                // Ended at the previous tick: gone now.
                e.running.retain(|r| !r.ended);
                let mut evs = Vec::new();
                for r in e.running.iter_mut() {
                    let ph = r.phase_at(now);
                    let name = props[r.prop].name;
                    let first = (-r.delay).max(0.0).min(r.duration);
                    let send_run = r.phase == Phase::Idle;
                    let send_start = matches!(r.phase, Phase::Idle | Phase::Before) && matches!(ph, Phase::Active | Phase::After);
                    let send_end = r.phase != Phase::After && ph == Phase::After;
                    if send_run {
                        evs.push(("transitionrun", name, first));
                    }
                    if send_start {
                        evs.push(("transitionstart", name, first));
                    }
                    if send_end {
                        evs.push(("transitionend", name, r.duration));
                        r.ended = true;
                        out.restyle = true;
                        soon(now, &mut next);
                    }
                    r.phase = ph;
                    match ph {
                        Phase::Before | Phase::Idle => soon(r.start + r.delay, &mut next),
                        Phase::Active => {
                            active = true;
                            soon(r.start + r.delay + r.duration, &mut next);
                        }
                        Phase::After => {}
                    }
                }
                out.events.extend(evs.into_iter().map(|(k, p, t)| ev(e, k, p, t)));
            }
        });
        s.retain(|_, e| e.declares || !e.running.is_empty());
        recount(&s);
    });
    if active && interval.is_finite() {
        let last = LAST_FRAME.with(Cell::get);
        if now - last >= interval {
            out.restyle = true;
            LAST_FRAME.with(|f| f.set(now));
            soon(now + interval, &mut next);
        } else {
            soon(last + interval, &mut next);
        }
    }
    out.next = next;
    out
}

/// Test seam: the running transitions of `(node, slot)` as `(property, start, delay, duration)`.
#[cfg(test)]
pub(crate) fn running_of(node: usize, pseudo: u8) -> Vec<(&'static str, f64, f64, f64)> {
    STORE.with(|s| {
        PROPS.with(|props| {
            s.borrow()
                .get(&(node, pseudo))
                .map(|e| e.running.iter().filter(|r| !r.ended).map(|r| (props[r.prop].name, r.start, r.delay, r.duration)).collect())
                .unwrap_or_default()
        })
    })
}
