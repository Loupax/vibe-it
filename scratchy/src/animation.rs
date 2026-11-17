//! QAN1a: CSS animations, PARSING ONLY. `@keyframes` rules go into a side
//! registry, the eight `animation-*` longhands and the `animation`
//! shorthand into `Declarations::anim`, and the cascaded per-element result
//! is an interned `&'static AnimLonghands` (so `ComputedStyle` stays `Copy`,
//! same trick as `custom_props`). Nothing evaluates any of it yet (QAN1b).
use super::*;

/// `steps()` jump terms. `step-start` is `JumpStart`, `step-end` `JumpEnd`.
#[derive(Clone, Copy, PartialEq, Debug)]
#[allow(clippy::enum_variant_names)] // the spec names: jump-start, jump-end, ...
pub(crate) enum StepPos {
    JumpStart,
    JumpEnd,
    JumpNone,
    JumpBoth,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum TimingFunction {
    Linear,
    CubicBezier(f32, f32, f32, f32),
    Steps(u32, StepPos),
}

impl TimingFunction {
    pub(crate) const EASE: TimingFunction = TimingFunction::CubicBezier(0.25, 0.1, 0.25, 1.0);
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum AnimDirection {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum AnimFill {
    None,
    Forwards,
    Backwards,
    Both,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum AnimPlayState {
    Running,
    Paused,
}

/// The declared lists of one declaration block, one entry per longhand;
/// `None` = not declared here. Times are milliseconds, an infinite
/// iteration count is `f32::INFINITY`, `None` in `name` is the keyword
/// `none`.
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct AnimDecls {
    pub(crate) name: Option<Vec<Option<String>>>,
    pub(crate) duration: Option<Vec<f32>>,
    pub(crate) timing: Option<Vec<TimingFunction>>,
    pub(crate) delay: Option<Vec<f32>>,
    pub(crate) iterations: Option<Vec<f32>>,
    pub(crate) direction: Option<Vec<AnimDirection>>,
    pub(crate) fill: Option<Vec<AnimFill>>,
    pub(crate) play_state: Option<Vec<AnimPlayState>>,
}

/// The computed, per-element animation lists (one `Vec` per longhand, an
/// empty one meaning the initial value). Interned: see `intern_anim`.
#[derive(Clone, Default, PartialEq, Debug)]
pub(crate) struct AnimLonghands {
    pub(crate) name: Vec<Option<String>>,
    pub(crate) duration: Vec<f32>,
    pub(crate) timing: Vec<TimingFunction>,
    pub(crate) delay: Vec<f32>,
    pub(crate) iterations: Vec<f32>,
    pub(crate) direction: Vec<AnimDirection>,
    pub(crate) fill: Vec<AnimFill>,
    pub(crate) play_state: Vec<AnimPlayState>,
}

/// One entry of the animation list after zipping the longhands
/// (`AnimLonghands::resolve`).
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Animation {
    pub(crate) name: Option<String>,
    pub(crate) duration: f32,
    pub(crate) timing: TimingFunction,
    pub(crate) delay: f32,
    pub(crate) iterations: f32,
    pub(crate) direction: AnimDirection,
    pub(crate) fill: AnimFill,
    pub(crate) play_state: AnimPlayState,
}

fn cyc<T: Clone>(v: &[T], i: usize, default: T) -> T {
    if v.is_empty() { default } else { v[i % v.len()].clone() }
}

impl AnimLonghands {
    /// CSS Animations 1 §3: the list is as long as `animation-name`'s, every
    /// other longhand repeats (or is truncated) to that length.
    #[allow(dead_code)] // QAN1b evaluates
    pub(crate) fn resolve(&self) -> Vec<Animation> {
        (0..self.name.len())
            .map(|i| Animation {
                name: self.name[i].clone(),
                duration: cyc(&self.duration, i, 0.0),
                timing: cyc(&self.timing, i, TimingFunction::EASE),
                delay: cyc(&self.delay, i, 0.0),
                iterations: cyc(&self.iterations, i, 1.0),
                direction: cyc(&self.direction, i, AnimDirection::Normal),
                fill: cyc(&self.fill, i, AnimFill::None),
                play_state: cyc(&self.play_state, i, AnimPlayState::Running),
            })
            .collect()
    }
}

thread_local! {
    static ANIM_INTERNER: RefCell<HashMap<String, &'static AnimLonghands>> = RefCell::new(HashMap::new());
}

/// Leaks one copy of each distinct list set (bounded by the number of
/// distinct values on a page). The default (nothing declared) is `None`.
pub(crate) fn intern_anim(a: AnimLonghands) -> Option<&'static AnimLonghands> {
    if a == AnimLonghands::default() {
        return None;
    }
    ANIM_INTERNER.with(|i| {
        let mut i = i.borrow_mut();
        let key = format!("{a:?}");
        Some(*i.entry(key).or_insert_with(|| Box::leak(Box::new(a))))
    })
}

pub(crate) const ANIMATION_KEYS: [&str; 8] = [
    "animation-name", "animation-duration", "animation-timing-function", "animation-delay",
    "animation-iteration-count", "animation-direction", "animation-fill-mode", "animation-play-state",
];

pub(crate) fn is_animation_property(name: &str) -> bool {
    name.eq_ignore_ascii_case("animation") || ANIMATION_KEYS.iter().any(|k| name.eq_ignore_ascii_case(k))
}

impl AnimDecls {
    /// Drops the declared value of one longhand (a CSS-wide keyword took over).
    pub(crate) fn clear(&mut self, key: &str) {
        match key {
            "animation-name" => self.name = None,
            "animation-duration" => self.duration = None,
            "animation-timing-function" => self.timing = None,
            "animation-delay" => self.delay = None,
            "animation-iteration-count" => self.iterations = None,
            "animation-direction" => self.direction = None,
            "animation-fill-mode" => self.fill = None,
            "animation-play-state" => self.play_state = None,
            _ => {}
        }
    }
}

type PErr = ParseError<()>;

fn parse_time(tok: &Token<'_>) -> Option<f32> {
    match tok {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("s") => Some(value * 1000.0),
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("ms") => Some(*value),
        _ => None,
    }
}

fn timing_keyword(id: &str) -> Option<TimingFunction> {
    Some(match_ignore_ascii_case! { id,
        "linear" => TimingFunction::Linear,
        "ease" => TimingFunction::EASE,
        "ease-in" => TimingFunction::CubicBezier(0.42, 0.0, 1.0, 1.0),
        "ease-out" => TimingFunction::CubicBezier(0.0, 0.0, 0.58, 1.0),
        "ease-in-out" => TimingFunction::CubicBezier(0.42, 0.0, 0.58, 1.0),
        "step-start" => TimingFunction::Steps(1, StepPos::JumpStart),
        "step-end" => TimingFunction::Steps(1, StepPos::JumpEnd),
        _ => return None,
    })
}

fn parse_timing<'i>(i: &mut CssParser<'i>) -> Result<TimingFunction, PErr> {
    let tok = i.next()?.clone();
    match tok {
        Token::Ident(id) => timing_keyword(&id).ok_or_else(|| i.new_error_for_next_token()),
        Token::Function(f) if f.eq_ignore_ascii_case("cubic-bezier") => i.parse_nested_block(|i| {
            let x1 = i.expect_number()?;
            i.expect_comma()?;
            let y1 = i.expect_number()?;
            i.expect_comma()?;
            let x2 = i.expect_number()?;
            i.expect_comma()?;
            let y2 = i.expect_number()?;
            i.expect_exhausted()?;
            if !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&x2) {
                return Err(i.new_error_for_next_token());
            }
            Ok(TimingFunction::CubicBezier(x1, y1, x2, y2))
        }),
        Token::Function(f) if f.eq_ignore_ascii_case("steps") => i.parse_nested_block(|i| {
            let n = match i.next()?.clone() {
                Token::Number { int_value: Some(n), .. } if n >= 1 => n as u32,
                _ => return Err(i.new_error_for_next_token()),
            };
            let pos = if i.try_parse(|i| i.expect_comma()).is_ok() {
                let id = i.expect_ident()?.to_ascii_lowercase();
                match id.as_str() {
                    "jump-start" | "start" => StepPos::JumpStart,
                    "jump-end" | "end" => StepPos::JumpEnd,
                    "jump-none" => StepPos::JumpNone,
                    "jump-both" => StepPos::JumpBoth,
                    _ => return Err(i.new_error_for_next_token()),
                }
            } else {
                StepPos::JumpEnd
            };
            i.expect_exhausted()?;
            if pos == StepPos::JumpNone && n < 2 {
                return Err(i.new_error_for_next_token());
            }
            Ok(TimingFunction::Steps(n, pos))
        }),
        _ => Err(i.new_error_for_next_token()),
    }
}

fn is_reserved_name(id: &str) -> bool {
    ["initial", "inherit", "unset", "revert", "revert-layer", "default"]
        .iter()
        .any(|k| id.eq_ignore_ascii_case(k))
}

fn parse_name<'i>(i: &mut CssParser<'i>) -> Result<Option<String>, PErr> {
    match i.next()?.clone() {
        Token::Ident(id) if id.eq_ignore_ascii_case("none") => Ok(None),
        Token::Ident(id) if !is_reserved_name(&id) => Ok(Some(id.to_string())),
        Token::QuotedString(s) => Ok(Some(s.to_string())),
        _ => Err(i.new_error_for_next_token()),
    }
}

fn parse_duration<'i>(i: &mut CssParser<'i>) -> Result<f32, PErr> {
    let t = i.next()?.clone();
    match parse_time(&t) {
        Some(v) if v >= 0.0 => Ok(v),
        _ => Err(i.new_error_for_next_token()),
    }
}

fn parse_delay<'i>(i: &mut CssParser<'i>) -> Result<f32, PErr> {
    let t = i.next()?.clone();
    parse_time(&t).ok_or_else(|| i.new_error_for_next_token())
}

fn parse_iterations<'i>(i: &mut CssParser<'i>) -> Result<f32, PErr> {
    match i.next()?.clone() {
        Token::Ident(id) if id.eq_ignore_ascii_case("infinite") => Ok(f32::INFINITY),
        Token::Number { value, .. } if value >= 0.0 => Ok(value),
        _ => Err(i.new_error_for_next_token()),
    }
}

fn direction_keyword(id: &str) -> Option<AnimDirection> {
    Some(match_ignore_ascii_case! { id,
        "normal" => AnimDirection::Normal,
        "reverse" => AnimDirection::Reverse,
        "alternate" => AnimDirection::Alternate,
        "alternate-reverse" => AnimDirection::AlternateReverse,
        _ => return None,
    })
}

fn fill_keyword(id: &str) -> Option<AnimFill> {
    Some(match_ignore_ascii_case! { id,
        "none" => AnimFill::None,
        "forwards" => AnimFill::Forwards,
        "backwards" => AnimFill::Backwards,
        "both" => AnimFill::Both,
        _ => return None,
    })
}

fn play_keyword(id: &str) -> Option<AnimPlayState> {
    Some(match_ignore_ascii_case! { id,
        "running" => AnimPlayState::Running,
        "paused" => AnimPlayState::Paused,
        _ => return None,
    })
}

fn ident_with<'i, T>(i: &mut CssParser<'i>, f: fn(&str) -> Option<T>) -> Result<T, PErr> {
    let id = i.expect_ident()?.clone();
    f(&id).ok_or_else(|| i.new_error_for_next_token())
}

/// One `<single-animation>` of the shorthand: the first `<time>` is the
/// duration, the second the delay, a keyword valid for another longhand is
/// that longhand's, whatever ident is left is the name. Every omitted part
/// takes its initial value.
fn parse_single<'i>(i: &mut CssParser<'i>) -> Result<Animation, PErr> {
    let (mut dur, mut delay, mut timing, mut iter) = (None, None, None, None);
    let (mut dir, mut fill, mut play) = (None, None, None);
    let mut name: Option<Option<String>> = None;
    let mut none_seen = false;
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
            Token::Number { value, .. } if iter.is_none() && value >= 0.0 => iter = Some(value),
            Token::Function(_) if timing.is_none() => {
                i.reset(&st);
                timing = Some(parse_timing(i)?);
            }
            Token::QuotedString(s) if name.is_none() => name = Some(Some(s.to_string())),
            Token::Ident(id) => {
                if id.eq_ignore_ascii_case("none") {
                    if name.is_none() && !none_seen {
                        none_seen = true;
                    } else if fill.is_none() {
                        fill = Some(AnimFill::None);
                    } else {
                        return Err(i.new_error_for_next_token());
                    }
                } else if id.eq_ignore_ascii_case("infinite") && iter.is_none() {
                    iter = Some(f32::INFINITY);
                } else if let (Some(t), true) = (timing_keyword(&id), timing.is_none()) {
                    timing = Some(t);
                } else if let (Some(d), true) = (direction_keyword(&id), dir.is_none()) {
                    dir = Some(d);
                } else if let (Some(f), true) = (fill_keyword(&id), fill.is_none()) {
                    fill = Some(f);
                } else if let (Some(p), true) = (play_keyword(&id), play.is_none()) {
                    play = Some(p);
                } else if name.is_none() && !is_reserved_name(&id) {
                    name = Some(Some(id.to_string()));
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
    if none_seen {
        if name.is_none() {
            name = Some(None);
        } else if fill.is_none() {
            fill = Some(AnimFill::None);
        } else {
            return Err(i.new_error_for_next_token());
        }
    }
    Ok(Animation {
        name: name.unwrap_or(None),
        duration: dur.unwrap_or(0.0),
        timing: timing.unwrap_or(TimingFunction::EASE),
        delay: delay.unwrap_or(0.0),
        iterations: iter.unwrap_or(1.0),
        direction: dir.unwrap_or(AnimDirection::Normal),
        fill: fill.unwrap_or(AnimFill::None),
        play_state: play.unwrap_or(AnimPlayState::Running),
    })
}

fn list<'i, T>(
    i: &mut CssParser<'i>,
    f: impl FnMut(&mut CssParser<'i>) -> Result<T, PErr>,
) -> Result<Vec<T>, PErr> {
    let v = i.parse_comma_separated(f)?;
    i.expect_exhausted()?;
    Ok(v)
}

/// Parses the (already `!important`-stripped) value of `name` (`animation`
/// or one longhand, unprefixed) into a fresh `AnimDecls`; `Err` = invalid,
/// the declaration is dropped.
pub(crate) fn parse_animation_value(name: &str, value: &str) -> Result<AnimDecls, ()> {
    let mut input = CssParser::new(value);
    let i = &mut input;
    let mut d = AnimDecls::default();
    let r: Result<(), PErr> = (|| {
        match name.to_ascii_lowercase().as_str() {
            "animation" => {
                let v = list(i, parse_single)?;
                d.name = Some(v.iter().map(|a| a.name.clone()).collect());
                d.duration = Some(v.iter().map(|a| a.duration).collect());
                d.timing = Some(v.iter().map(|a| a.timing).collect());
                d.delay = Some(v.iter().map(|a| a.delay).collect());
                d.iterations = Some(v.iter().map(|a| a.iterations).collect());
                d.direction = Some(v.iter().map(|a| a.direction).collect());
                d.fill = Some(v.iter().map(|a| a.fill).collect());
                d.play_state = Some(v.iter().map(|a| a.play_state).collect());
            }
            "animation-name" => d.name = Some(list(i, parse_name)?),
            "animation-duration" => d.duration = Some(list(i, parse_duration)?),
            "animation-timing-function" => d.timing = Some(list(i, parse_timing)?),
            "animation-delay" => d.delay = Some(list(i, parse_delay)?),
            "animation-iteration-count" => d.iterations = Some(list(i, parse_iterations)?),
            "animation-direction" => d.direction = Some(list(i, |i| ident_with(i, direction_keyword))?),
            "animation-fill-mode" => d.fill = Some(list(i, |i| ident_with(i, fill_keyword))?),
            "animation-play-state" => d.play_state = Some(list(i, |i| ident_with(i, play_keyword))?),
            _ => return Err(i.new_error_for_next_token()),
        }
        Ok(())
    })();
    r.map(|_| d).map_err(|_| ())
}

impl Declarations {
    /// Merges a parsed animation declaration (later wins per longhand).
    pub(crate) fn set_animation(&mut self, d: AnimDecls) {
        let a = self.anim.get_or_insert_with(Default::default);
        macro_rules! m { ($($f:ident),*) => { $(if d.$f.is_some() { a.$f = d.$f; })* }; }
        m!(name, duration, timing, delay, iterations, direction, fill, play_state);
    }

    pub(crate) fn clear_animation(&mut self, key: &str) {
        if let Some(a) = self.anim.as_mut() {
            a.clear(key);
        }
    }
}

impl DeclParser<'_> {
    /// `animation` and its longhands (already un-prefixed by the caller).
    pub(crate) fn apply_animation(&mut self, name: &str, input: &mut CssParser<'_>) {
        let start = input.state();
        while input.next().is_ok() {}
        let raw = input.slice_from(start.position()).trim();
        let raw = strip_important(raw).unwrap_or(raw);
        if let Ok(d) = parse_animation_value(name, raw) {
            self.decls.set_animation(d);
        }
    }
}

/// The per-element cascade of the eight longhands. Keyword winners
/// (`inherit`/`initial`) are left empty here and fixed by `apply_rest_wide`
/// (`copy_prop`), exactly like every other non-core property.
pub(crate) fn cascade_animations(
    matches: &[(u64, &Declarations)],
    inline: Option<&Declarations>,
) -> Option<&'static AnimLonghands> {
    if !(matches.iter().any(|(_, d)| d.anim.is_some()) || inline.is_some_and(|d| d.anim.is_some())) {
        return None;
    }
    macro_rules! pick {
        ($key:literal, $f:ident) => {
            match cascade_css(matches, inline, $key, |d| d.anim.as_ref().and_then(|a| a.$f.clone())) {
                Cascaded::Value(v) => v,
                _ => Vec::new(),
            }
        };
    }
    intern_anim(AnimLonghands {
        name: pick!("animation-name", name),
        duration: pick!("animation-duration", duration),
        timing: pick!("animation-timing-function", timing),
        delay: pick!("animation-delay", delay),
        iterations: pick!("animation-iteration-count", iterations),
        direction: pick!("animation-direction", direction),
        fill: pick!("animation-fill-mode", fill),
        play_state: pick!("animation-play-state", play_state),
    })
}

/// `inherit`/`initial` of one longhand: copies that longhand's list from
/// `src` (the parent, or the initial style).
pub(crate) fn copy_animation_longhand(
    dst: &mut Option<&'static AnimLonghands>,
    src: Option<&'static AnimLonghands>,
    key: &str,
) {
    let mut a = dst.cloned().unwrap_or_default();
    let s = src.cloned().unwrap_or_default();
    match key {
        "animation-name" => a.name = s.name,
        "animation-duration" => a.duration = s.duration,
        "animation-timing-function" => a.timing = s.timing,
        "animation-delay" => a.delay = s.delay,
        "animation-iteration-count" => a.iterations = s.iterations,
        "animation-direction" => a.direction = s.direction,
        "animation-fill-mode" => a.fill = s.fill,
        "animation-play-state" => a.play_state = s.play_state,
        _ => return,
    }
    *dst = intern_anim(a);
}

/// One keyframe: an offset in 0..=1 and the declarations written for it.
/// `!important` declarations were dropped at parse time (spec); an
/// `animation-timing-function` in `decls.anim` is the keyframe's own easing
/// (QAN1b), every other `animation-*` is ignored by the evaluator.
#[derive(Clone, PartialEq)]
pub(crate) struct Keyframe {
    pub(crate) offset: f32,
    pub(crate) decls: Declarations,
}

/// `@keyframes` name -> keyframes sorted by offset (stable: source order
/// among equal offsets); the last definition of a name wins.
/// A definition inside an `@media` carries its (unevaluated) query list:
/// the viewport can change after parsing, so it is checked at lookup.
pub(crate) type KeyframesRegistry = HashMap<String, Vec<(Vec<MediaQuery>, Rc<Vec<Keyframe>>)>>;

/// The last definition of `name` whose enclosing `@media` holds.
pub(crate) fn registry_get(reg: &KeyframesRegistry, name: &str, viewport_width: f32) -> Option<Rc<Vec<Keyframe>>> {
    reg.get(name)?.iter().rev().find(|(m, _)| media_list_matches(m, viewport_width)).map(|(_, k)| k.clone())
}

thread_local! {
    static KEYFRAMES: RefCell<Rc<KeyframesRegistry>> = RefCell::new(Rc::new(HashMap::new()));
}

/// Installs the registry of the sheet in use (`build_layout_tree_with_resources`).
pub(crate) fn set_keyframes(r: Rc<KeyframesRegistry>) {
    KEYFRAMES.with(|k| *k.borrow_mut() = r);
}

#[allow(dead_code)] // QAN1b evaluates
pub(crate) fn lookup_keyframes(name: &str, viewport_width: f32) -> Option<Rc<Vec<Keyframe>>> {
    KEYFRAMES.with(|k| registry_get(&k.borrow(), name, viewport_width))
}

/// `@keyframes` prelude: one ident (not `none` or a CSS-wide keyword) or a string.
pub(crate) fn parse_keyframes_name<'i>(i: &mut CssParser<'i>) -> Result<String, ParseError<CssError>> {
    let name = match i.next()?.clone() {
        Token::Ident(id) if !id.eq_ignore_ascii_case("none") && !is_reserved_name(&id) => id.to_string(),
        Token::QuotedString(s) => s.to_string(),
        _ => return Err(i.new_error_for_next_token()),
    };
    i.expect_exhausted()?;
    Ok(name)
}

/// The rules inside one `@keyframes` block.
pub(crate) struct KeyframeBlockParser;

impl<'i> AtRuleParser<'i> for KeyframeBlockParser {
    type Prelude = ();
    type AtRule = Vec<Keyframe>;
    type Error = CssError;
}

impl<'i> QualifiedRuleParser<'i> for KeyframeBlockParser {
    type Prelude = Vec<f32>;
    type QualifiedRule = Vec<Keyframe>;
    type Error = CssError;

    fn parse_prelude(&mut self, input: &mut CssParser<'i>) -> Result<Vec<f32>, ParseError<CssError>> {
        input.parse_comma_separated(|i| match i.next()?.clone() {
            Token::Ident(id) if id.eq_ignore_ascii_case("from") => Ok(0.0),
            Token::Ident(id) if id.eq_ignore_ascii_case("to") => Ok(1.0),
            Token::Percentage { unit_value, .. } if (0.0..=1.0).contains(&unit_value) => Ok(unit_value),
            _ => Err(i.new_error_for_next_token()),
        })
    }

    fn parse_block(
        &mut self,
        prelude: Vec<f32>,
        _start: &ParserState,
        input: &mut CssParser<'i>,
    ) -> Result<Vec<Keyframe>, ParseError<CssError>> {
        let mut decls = Declarations::default();
        let mut important = Declarations::default();
        {
            let mut p = DeclParser { decls: &mut decls, important: &mut important };
            let mut iter = RuleBodyParser::new(input, &mut p);
            while iter.next().is_some() {}
        }
        // `important` is dropped: `!important` in a keyframe is ignored.
        Ok(prelude.into_iter().map(|offset| Keyframe { offset, decls: decls.clone() }).collect())
    }
}

/// Parses the block of `@keyframes name { ... }` into its sorted keyframes.
pub(crate) fn parse_keyframes_block(input: &mut CssParser<'_>) -> Vec<Keyframe> {
    let mut out = Vec::new();
    let mut p = KeyframeBlockParser;
    for r in StyleSheetParser::new(input, &mut p).flatten() {
        out.extend(r);
    }
    out.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    out
}

/// Builds the registry from `(name, keyframes)` pairs in source order.
pub(crate) fn build_registry(list: Vec<KeyframesDef>) -> KeyframesRegistry {
    let mut reg = KeyframesRegistry::new();
    for (n, m, k) in list {
        reg.entry(n).or_default().push((m, Rc::new(k)));
    }
    reg
}

/// One parsed `@keyframes`: name, enclosing media queries, keyframes.
pub(crate) type KeyframesDef = (String, Vec<MediaQuery>, Vec<Keyframe>);
