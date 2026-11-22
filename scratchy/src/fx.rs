//! QCSS-D: `filter`, `backdrop-filter`, `text-shadow`, `mix-blend-mode` and `clip-path` basic shapes.
//!
//! This file holds the value types (declared form with unresolved `Length`s, computed form in px),
//! the parsers, the computed-value serialiser used by `getComputedStyle`, and the pixel code.
//!
//! The page buffer is opaque (`0x00RRGGBB`, no alpha), so an element that needs its own alpha (a
//! filter, `mix-blend-mode`) is painted twice into scratch buffers, once over black and once over
//! white; per channel `white - black = 255 * (1 - alpha)` and `black = colour * alpha`, which gives
//! the premultiplied RGBA of the subtree. The filter chain runs on that layer and the result is
//! composited onto the page. `clip-path` alone only needs a before/after snapshot of the region.
//! `backdrop-filter` filters the page pixels under the border box before the box paints.
//!
//! Cost bounds: the layer covers the box's ink plus the filter's reach, clipped to the window; a
//! region over `MAX_REGION_PX` pixels is painted without the effect, a blur's sigma is capped at
//! `MAX_SIGMA`, and the number of functions at `MAX_FILTER_FNS`.
use super::*;

/// Values the stylesheet declares: `none` or a value. A plain `Option<T>` could not tell an explicit
/// `none` (which must win the cascade) from "not declared" (same reasoning as `ShadowDecl`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Fx<T> {
    None,
    Value(T),
}

/// One `<filter-function>`. `L` is the length type (`Length` declared, `f32` px computed) and `C`
/// the colour type (`Option<CssColor>` declared, `TextColor` computed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum FilterFn<L, C> {
    Blur(L),
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    Invert(f32),
    Opacity(f32),
    Saturate(f32),
    Sepia(f32),
    /// Degrees.
    HueRotate(f32),
    DropShadow(Shadow<L, C>),
}

/// A `text-shadow` / `drop-shadow()` layer: `<color>? && <length>{2,3}`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct Shadow<L, C> {
    pub(crate) dx: L,
    pub(crate) dy: L,
    pub(crate) blur: L,
    pub(crate) color: C,
}

pub(crate) type FilterDeclFn = FilterFn<Length, Option<CssColor>>;
pub(crate) type FilterCompFn = FilterFn<f32, TextColor>;
pub(crate) type ShadowDeclFn = Shadow<Length, Option<CssColor>>;
pub(crate) type ShadowCompFn = Shadow<f32, TextColor>;

/// A `<length-percentage>` in px plus a fraction of the reference box.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) struct LP {
    pub(crate) px: f32,
    pub(crate) pct: f32,
}

impl LP {
    pub(crate) fn at(self, basis: f32) -> f32 {
        self.px + self.pct * basis
    }
}

/// `<shape-radius>`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Rad<L> {
    Len(L),
    Closest,
    Farthest,
}

/// A `<basic-shape>`; `L` is `Length` declared, `LP` computed. `inset()` keeps one corner radius
/// for all four corners (the first value of `round`).
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Shape<L> {
    Inset { t: L, r: L, b: L, l: L, round: L },
    Circle { r: Rad<L>, cx: L, cy: L },
    Ellipse { rx: Rad<L>, ry: Rad<L>, cx: L, cy: L },
    Polygon { evenodd: bool, pts: Vec<(L, L)> },
}

/// `mix-blend-mode`. The four non-separable modes parse and serialise but paint as `normal`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

const BLEND_NAMES: [(&str, BlendMode); 16] = [
    ("normal", BlendMode::Normal),
    ("multiply", BlendMode::Multiply),
    ("screen", BlendMode::Screen),
    ("overlay", BlendMode::Overlay),
    ("darken", BlendMode::Darken),
    ("lighten", BlendMode::Lighten),
    ("color-dodge", BlendMode::ColorDodge),
    ("color-burn", BlendMode::ColorBurn),
    ("hard-light", BlendMode::HardLight),
    ("soft-light", BlendMode::SoftLight),
    ("difference", BlendMode::Difference),
    ("exclusion", BlendMode::Exclusion),
    ("hue", BlendMode::Hue),
    ("saturation", BlendMode::Saturation),
    ("color", BlendMode::Color),
    ("luminosity", BlendMode::Luminosity),
];

impl BlendMode {
    pub(crate) fn from_keyword(k: &str) -> Option<BlendMode> {
        BLEND_NAMES.iter().find(|(n, _)| k.eq_ignore_ascii_case(n)).map(|(_, m)| *m)
    }
    pub(crate) fn keyword(self) -> &'static str {
        BLEND_NAMES.iter().find(|(_, m)| *m == self).map_or("normal", |(n, _)| n)
    }
    fn separable(self) -> bool {
        !matches!(self, BlendMode::Normal | BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity)
    }
}

pub(crate) const MAX_FILTER_FNS: usize = 8;
pub(crate) const MAX_TEXT_SHADOWS: usize = 6;
/// A blur's standard deviation is capped here (px).
pub(crate) const MAX_SIGMA: f32 = 40.0;
/// A layer region (ink plus the filter's reach, clipped to the window) over this many pixels is
/// painted without the effect.
pub(crate) const MAX_REGION_PX: i64 = 2_400_000;

// ---------------------------------------------------------------------------------------------
// Interning: ComputedStyle is `Copy`, so lists live in leaked, de-duplicated tables.
// ---------------------------------------------------------------------------------------------

fn intern<T: PartialEq + Send + Sync + 'static>(tbl: &'static Mutex<Vec<&'static T>>, v: T) -> &'static T {
    let mut t = tbl.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = t.iter().find(|r| ***r == v) {
        return r;
    }
    let r: &'static T = Box::leak(Box::new(v));
    t.push(r);
    r
}

fn intern_list<T: PartialEq + Send + Sync + 'static>(tbl: &'static Mutex<Vec<&'static [T]>>, v: Vec<T>) -> &'static [T] {
    let mut t = tbl.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = t.iter().find(|r| **r == v.as_slice()) {
        return r;
    }
    let r: &'static [T] = Box::leak(v.into_boxed_slice());
    t.push(r);
    r
}

static T_FILTER_DECL: Mutex<Vec<&'static [FilterDeclFn]>> = Mutex::new(Vec::new());
static T_FILTER_COMP: Mutex<Vec<&'static [FilterCompFn]>> = Mutex::new(Vec::new());
static T_SHADOW_DECL: Mutex<Vec<&'static [ShadowDeclFn]>> = Mutex::new(Vec::new());
static T_SHADOW_COMP: Mutex<Vec<&'static [ShadowCompFn]>> = Mutex::new(Vec::new());
static T_SHAPE_DECL: Mutex<Vec<&'static Shape<Length>>> = Mutex::new(Vec::new());
static T_SHAPE_COMP: Mutex<Vec<&'static Shape<LP>>> = Mutex::new(Vec::new());

// ---------------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------------

/// True at the end of the value, or at the `!` of `!important`.
fn at_end(input: &mut CssParser) -> bool {
    let st = input.state();
    let r = matches!(input.next(), Err(_) | Ok(Token::Delim('!')));
    input.reset(&st);
    r
}

fn number_or_pct(input: &mut CssParser) -> Result<f32, ()> {
    match input.next().map_err(|_| ())? {
        Token::Number { value, .. } => Ok(*value),
        Token::Percentage { unit_value, .. } => Ok(*unit_value),
        _ => Err(()),
    }
}

fn parse_angle(input: &mut CssParser) -> Result<f32, ()> {
    match input.next().map_err(|_| ())? {
        Token::Dimension { value, unit, .. } => {
            let u = unit.to_ascii_lowercase();
            match u.as_str() {
                "deg" => Ok(*value),
                "grad" => Ok(value * 0.9),
                "rad" => Ok(value.to_degrees()),
                "turn" => Ok(value * 360.0),
                _ => Err(()),
            }
        }
        Token::Number { value, .. } if *value == 0.0 => Ok(0.0),
        _ => Err(()),
    }
}

/// `<color>? && <length>{2,3}` up to a comma, `!` or the end of `input`.
fn parse_shadow_parts(input: &mut CssParser) -> Result<ShadowDeclFn, ()> {
    let mut lens: Vec<Length> = Vec::new();
    let mut color: Option<CssColor> = None;
    loop {
        let st = input.state();
        if let Ok(l) = parse_length(input) {
            lens.push(l);
            continue;
        }
        input.reset(&st);
        if color.is_none() {
            if let Ok(c) = CssColor::parse(input) {
                color = Some(c);
                continue;
            }
            input.reset(&st);
        }
        break;
    }
    if lens.len() < 2 || lens.len() > 3 {
        return Err(());
    }
    if matches!(lens.get(2), Some(Length::Px(v)) if *v < 0.0) {
        return Err(());
    }
    Ok(Shadow { dx: lens[0], dy: lens[1], blur: lens.get(2).copied().unwrap_or(Length::Px(0.0)), color })
}

fn nested<T>(input: &mut CssParser, f: impl FnOnce(&mut CssParser) -> Result<T, ()>) -> Result<T, ()> {
    let r: Result<T, ParseError<()>> = input.parse_nested_block(|i| {
        let v = f(i).map_err(|_| i.new_error_for_next_token::<()>())?;
        if !i.is_exhausted() {
            return Err(i.new_error_for_next_token::<()>());
        }
        Ok(v)
    });
    r.map_err(|_| ())
}

fn parse_filter_fn(input: &mut CssParser) -> Result<FilterDeclFn, ()> {
    let name = match input.next().map_err(|_| ())? {
        Token::Function(n) => n.to_ascii_lowercase(),
        _ => return Err(()),
    };
    nested(input, |i| {
        // The amount of the colour functions: `<number> | <percentage>`, default 1, never negative.
        let amount = |i: &mut CssParser| -> Result<f32, ()> {
            if i.is_exhausted() {
                return Ok(1.0);
            }
            let v = number_or_pct(i)?;
            if v < 0.0 { Err(()) } else { Ok(v) }
        };
        Ok(match name.as_str() {
            "blur" => {
                if i.is_exhausted() {
                    FilterFn::Blur(Length::Px(0.0))
                } else {
                    let l = parse_length(i)?;
                    if matches!(l, Length::Px(v) if v < 0.0) {
                        return Err(());
                    }
                    FilterFn::Blur(l)
                }
            }
            "brightness" => FilterFn::Brightness(amount(i)?),
            "contrast" => FilterFn::Contrast(amount(i)?),
            "grayscale" => FilterFn::Grayscale(amount(i)?),
            "invert" => FilterFn::Invert(amount(i)?),
            "opacity" => FilterFn::Opacity(amount(i)?),
            "saturate" => FilterFn::Saturate(amount(i)?),
            "sepia" => FilterFn::Sepia(amount(i)?),
            "hue-rotate" => FilterFn::HueRotate(if i.is_exhausted() { 0.0 } else { parse_angle(i)? }),
            "drop-shadow" => FilterFn::DropShadow(parse_shadow_parts(i)?),
            _ => return Err(()),
        })
    })
}

/// `none | <filter-function>+` (`url()` references are out of scope: the declaration is dropped).
pub(crate) fn parse_filter(input: &mut CssParser) -> Result<Fx<&'static [FilterDeclFn]>, ()> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return if at_end(input) { Ok(Fx::None) } else { Err(()) };
    }
    let mut v = Vec::new();
    while !at_end(input) {
        v.push(parse_filter_fn(input)?);
        if v.len() > MAX_FILTER_FNS {
            return Err(());
        }
    }
    if v.is_empty() {
        return Err(());
    }
    Ok(Fx::Value(intern_list(&T_FILTER_DECL, v)))
}

/// `none | <shadow>#`.
pub(crate) fn parse_text_shadow(input: &mut CssParser) -> Result<Fx<&'static [ShadowDeclFn]>, ()> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return if at_end(input) { Ok(Fx::None) } else { Err(()) };
    }
    let mut v = Vec::new();
    loop {
        v.push(parse_shadow_parts(input)?);
        if v.len() > MAX_TEXT_SHADOWS {
            return Err(());
        }
        if input.try_parse(|i| i.expect_comma()).is_err() {
            break;
        }
    }
    if !at_end(input) {
        return Err(());
    }
    Ok(Fx::Value(intern_list(&T_SHADOW_DECL, v)))
}

pub(crate) fn parse_mix_blend_mode(input: &mut CssParser) -> Result<BlendMode, ()> {
    let id = input.expect_ident().map_err(|_| ())?.clone();
    let m = BlendMode::from_keyword(&id).ok_or(())?;
    if at_end(input) { Ok(m) } else { Err(()) }
}

/// `closest-side | farthest-side | <length-percentage [0,)>`.
fn parse_rad(input: &mut CssParser) -> Result<Rad<Length>, ()> {
    if let Ok(id) = input.try_parse(|i| i.expect_ident().cloned()) {
        let k = id.to_ascii_lowercase();
        return match k.as_str() {
            "closest-side" => Ok(Rad::Closest),
            "farthest-side" => Ok(Rad::Farthest),
            _ => Err(()),
        };
    }
    let l = parse_length_percent(input)?;
    if matches!(l, Length::Px(v) | Length::Percent(v) if v < 0.0) {
        return Err(());
    }
    Ok(Rad::Len(l))
}

/// `<position>` after `at`: one or two items (keywords and length-percentages). The four-value
/// edge-offset form is not supported (the declaration is dropped).
fn parse_at(input: &mut CssParser) -> Result<(Length, Length), ()> {
    #[derive(Clone, Copy)]
    enum It {
        Left,
        Right,
        Top,
        Bottom,
        Center,
        Val(Length),
    }
    let mut items: Vec<It> = Vec::new();
    while items.len() < 3 && !at_end(input) {
        let st = input.state();
        if let Ok(Token::Ident(id)) = input.next().cloned() {
            let k = id.to_ascii_lowercase();
            items.push(match k.as_str() {
                "left" => It::Left,
                "right" => It::Right,
                "top" => It::Top,
                "bottom" => It::Bottom,
                "center" => It::Center,
                _ => return Err(()),
            });
            continue;
        }
        input.reset(&st);
        items.push(It::Val(parse_length_percent(input)?));
    }
    let to_len = |it: It| match it {
        It::Left | It::Top => Length::Px(0.0),
        It::Right | It::Bottom => Length::Percent(1.0),
        It::Center => Length::Percent(0.5),
        It::Val(l) => l,
    };
    let half = Length::Percent(0.5);
    match items.as_slice() {
        [] => Ok((half, half)),
        [a] => match a {
            It::Top | It::Bottom => Ok((half, to_len(*a))),
            _ => Ok((to_len(*a), half)),
        },
        [a, b] => {
            let swap = matches!(a, It::Top | It::Bottom) || matches!(b, It::Left | It::Right);
            let (h, v) = if swap { (*b, *a) } else { (*a, *b) };
            if matches!(h, It::Top | It::Bottom) || matches!(v, It::Left | It::Right) {
                return Err(());
            }
            Ok((to_len(h), to_len(v)))
        }
        _ => Err(()),
    }
}

fn parse_shape_fn(input: &mut CssParser) -> Result<Shape<Length>, ()> {
    let name = match input.next().map_err(|_| ())? {
        Token::Function(n) => n.to_ascii_lowercase(),
        _ => return Err(()),
    };
    nested(input, |i| {
        let zero = Length::Px(0.0);
        let half = Length::Percent(0.5);
        match name.as_str() {
            "inset" => {
                let mut v: Vec<Length> = Vec::new();
                while v.len() < 4 {
                    let st = i.state();
                    match parse_length_percent(i) {
                        Ok(l) => v.push(l),
                        Err(()) => {
                            i.reset(&st);
                            break;
                        }
                    }
                }
                let (t, r, b, l) = match v.as_slice() {
                    [a] => (*a, *a, *a, *a),
                    [a, b] => (*a, *b, *a, *b),
                    [a, b, c] => (*a, *b, *c, *b),
                    [a, b, c, d] => (*a, *b, *c, *d),
                    _ => return Err(()),
                };
                let mut round = zero;
                if i.try_parse(|i| i.expect_ident_matching("round")).is_ok() {
                    round = parse_length_percent(i)?;
                    // Further radii (and a `/ ...` second set) are accepted and ignored.
                    while !i.is_exhausted() {
                        i.next().map_err(|_| ())?;
                    }
                }
                Ok(Shape::Inset { t, r, b, l, round })
            }
            "circle" => {
                let r = i.try_parse(parse_rad).unwrap_or(Rad::Closest);
                let (cx, cy) = if i.try_parse(|i| i.expect_ident_matching("at")).is_ok() { parse_at(i)? } else { (half, half) };
                Ok(Shape::Circle { r, cx, cy })
            }
            "ellipse" => {
                let (rx, ry) = match i.try_parse(parse_rad) {
                    Ok(rx) => (rx, parse_rad(i)?),
                    Err(_) => (Rad::Closest, Rad::Closest),
                };
                let (cx, cy) = if i.try_parse(|i| i.expect_ident_matching("at")).is_ok() { parse_at(i)? } else { (half, half) };
                Ok(Shape::Ellipse { rx, ry, cx, cy })
            }
            "polygon" => {
                let mut evenodd = false;
                if let Ok(id) = i.try_parse(|i| i.expect_ident().cloned()) {
                    if id.eq_ignore_ascii_case("evenodd") {
                        evenodd = true;
                    } else if !id.eq_ignore_ascii_case("nonzero") {
                        return Err(());
                    }
                    i.expect_comma().map_err(|_| ())?;
                }
                let mut pts = Vec::new();
                loop {
                    let x = parse_length_percent(i)?;
                    let y = parse_length_percent(i)?;
                    pts.push((x, y));
                    if pts.len() > 256 {
                        return Err(());
                    }
                    if i.try_parse(|i| i.expect_comma()).is_err() {
                        break;
                    }
                }
                Ok(Shape::Polygon { evenodd, pts })
            }
            _ => Err(()),
        }
    })
}

fn is_geometry_box(id: &str) -> bool {
    matches!(
        id.to_ascii_lowercase().as_str(),
        "border-box" | "padding-box" | "content-box" | "margin-box" | "fill-box" | "stroke-box" | "view-box"
    )
}

/// `none | <basic-shape> || <geometry-box>`. The reference box is always the border box; a
/// geometry-box keyword alone clips to that box, which clips nothing. `url()` and `path()` drop the
/// declaration.
pub(crate) fn parse_clip_path(input: &mut CssParser) -> Result<Fx<&'static Shape<Length>>, ()> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return if at_end(input) { Ok(Fx::None) } else { Err(()) };
    }
    let mut shape = None;
    let mut boxes = 0;
    while !at_end(input) {
        let st = input.state();
        if let Ok(Token::Ident(id)) = input.next().cloned() {
            if is_geometry_box(&id) && boxes == 0 {
                boxes += 1;
                continue;
            }
            return Err(());
        }
        input.reset(&st);
        if shape.is_some() {
            return Err(());
        }
        shape = Some(parse_shape_fn(input)?);
    }
    match shape {
        Some(s) => Ok(Fx::Value(intern(&T_SHAPE_DECL, s))),
        None if boxes == 1 => Ok(Fx::None),
        None => Err(()),
    }
}

// ---------------------------------------------------------------------------------------------
// Resolution (declared -> computed)
// ---------------------------------------------------------------------------------------------

fn lp_of(l: Length, fs: f32, rfs: f32) -> LP {
    match l {
        Length::Percent(p) => LP { px: 0.0, pct: p },
        other => LP { px: other.resolve(fs, rfs), pct: 0.0 },
    }
}

fn color_of(c: Option<CssColor>, current: TextColor) -> TextColor {
    match c {
        None | Some(CssColor::CurrentColor) => current,
        Some(c) => to_text_color(&c).unwrap_or(current),
    }
}

fn shadow_of(s: &ShadowDeclFn, fs: f32, rfs: f32, current: TextColor) -> ShadowCompFn {
    Shadow { dx: s.dx.resolve(fs, rfs), dy: s.dy.resolve(fs, rfs), blur: s.blur.resolve(fs, rfs).max(0.0), color: color_of(s.color, current) }
}

pub(crate) fn resolve_filter(d: Fx<&'static [FilterDeclFn]>, fs: f32, rfs: f32, current: TextColor) -> Option<&'static [FilterCompFn]> {
    let Fx::Value(list) = d else { return None };
    let out: Vec<FilterCompFn> = list
        .iter()
        .map(|f| match f {
            FilterFn::Blur(l) => FilterFn::Blur(l.resolve(fs, rfs).max(0.0)),
            FilterFn::Brightness(v) => FilterFn::Brightness(*v),
            FilterFn::Contrast(v) => FilterFn::Contrast(*v),
            FilterFn::Grayscale(v) => FilterFn::Grayscale(*v),
            FilterFn::Invert(v) => FilterFn::Invert(*v),
            FilterFn::Opacity(v) => FilterFn::Opacity(*v),
            FilterFn::Saturate(v) => FilterFn::Saturate(*v),
            FilterFn::Sepia(v) => FilterFn::Sepia(*v),
            FilterFn::HueRotate(v) => FilterFn::HueRotate(*v),
            FilterFn::DropShadow(s) => FilterFn::DropShadow(shadow_of(s, fs, rfs, current)),
        })
        .collect();
    Some(intern_list(&T_FILTER_COMP, out))
}

pub(crate) fn resolve_text_shadow(d: Fx<&'static [ShadowDeclFn]>, fs: f32, rfs: f32, current: TextColor) -> Option<&'static [ShadowCompFn]> {
    let Fx::Value(list) = d else { return None };
    let out: Vec<ShadowCompFn> = list.iter().map(|s| shadow_of(s, fs, rfs, current)).collect();
    Some(intern_list(&T_SHADOW_COMP, out))
}

pub(crate) fn resolve_clip_path(d: Fx<&'static Shape<Length>>, fs: f32, rfs: f32) -> Option<&'static Shape<LP>> {
    let Fx::Value(s) = d else { return None };
    let lp = |l: &Length| lp_of(*l, fs, rfs);
    let rad = |r: &Rad<Length>| match r {
        Rad::Len(l) => Rad::Len(lp(l)),
        Rad::Closest => Rad::Closest,
        Rad::Farthest => Rad::Farthest,
    };
    let out = match s {
        Shape::Inset { t, r, b, l, round } => Shape::Inset { t: lp(t), r: lp(r), b: lp(b), l: lp(l), round: lp(round) },
        Shape::Circle { r, cx, cy } => Shape::Circle { r: rad(r), cx: lp(cx), cy: lp(cy) },
        Shape::Ellipse { rx, ry, cx, cy } => Shape::Ellipse { rx: rad(rx), ry: rad(ry), cx: lp(cx), cy: lp(cy) },
        Shape::Polygon { evenodd, pts } => Shape::Polygon { evenodd: *evenodd, pts: pts.iter().map(|(x, y)| (lp(x), lp(y))).collect() },
    };
    Some(intern(&T_SHAPE_COMP, out))
}

// ---------------------------------------------------------------------------------------------
// Serialisation (getComputedStyle)
// ---------------------------------------------------------------------------------------------

fn n(v: f32) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

fn px_s(v: f32) -> String {
    format!("{}px", n(v))
}

fn col_s(c: TextColor) -> String {
    if c.a() == 255 {
        format!("rgb({}, {}, {})", c.r(), c.g(), c.b())
    } else {
        format!("rgba({}, {}, {}, {})", c.r(), c.g(), c.b(), n(c.a() as f32 / 255.0))
    }
}

fn lp_s(l: LP) -> String {
    match (l.px == 0.0, l.pct == 0.0) {
        (_, true) => px_s(l.px),
        (true, false) => format!("{}%", n(l.pct * 100.0)),
        _ => format!("calc({}% + {})", n(l.pct * 100.0), px_s(l.px)),
    }
}

fn serialize_shadow(s: &ShadowCompFn) -> String {
    format!("{} {} {} {}", col_s(s.color), px_s(s.dx), px_s(s.dy), px_s(s.blur))
}

pub(crate) fn serialize_text_shadow(v: Option<&'static [ShadowCompFn]>) -> String {
    match v {
        None => "none".into(),
        Some(l) => l.iter().map(serialize_shadow).collect::<Vec<_>>().join(", "),
    }
}

pub(crate) fn serialize_filter(v: Option<&'static [FilterCompFn]>) -> String {
    let Some(l) = v else { return "none".into() };
    l.iter()
        .map(|f| match f {
            FilterFn::Blur(v) => format!("blur({})", px_s(*v)),
            FilterFn::Brightness(v) => format!("brightness({})", n(*v)),
            FilterFn::Contrast(v) => format!("contrast({})", n(*v)),
            FilterFn::Grayscale(v) => format!("grayscale({})", n(*v)),
            FilterFn::Invert(v) => format!("invert({})", n(*v)),
            FilterFn::Opacity(v) => format!("opacity({})", n(*v)),
            FilterFn::Saturate(v) => format!("saturate({})", n(*v)),
            FilterFn::Sepia(v) => format!("sepia({})", n(*v)),
            FilterFn::HueRotate(v) => format!("hue-rotate({}deg)", n(*v)),
            FilterFn::DropShadow(s) => format!("drop-shadow({})", serialize_shadow(s)),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn rad_s(r: &Rad<LP>) -> String {
    match r {
        Rad::Len(l) => lp_s(*l),
        Rad::Closest => "closest-side".into(),
        Rad::Farthest => "farthest-side".into(),
    }
}

pub(crate) fn serialize_clip_path(v: Option<&'static Shape<LP>>) -> String {
    let Some(s) = v else { return "none".into() };
    let at = |x: &LP, y: &LP| format!(" at {} {}", lp_s(*x), lp_s(*y));
    match s {
        Shape::Inset { t, r, b, l, round } => {
            let mut sides = vec![lp_s(*t), lp_s(*r), lp_s(*b), lp_s(*l)];
            if sides[3] == sides[1] {
                sides.pop();
                if sides[2] == sides[0] {
                    sides.pop();
                    if sides[1] == sides[0] {
                        sides.pop();
                    }
                }
            }
            let r = if *round == LP::default() { String::new() } else { format!(" round {}", lp_s(*round)) };
            format!("inset({}{})", sides.join(" "), r)
        }
        Shape::Circle { r, cx, cy } => format!("circle({}{})", rad_s(r), at(cx, cy)),
        Shape::Ellipse { rx, ry, cx, cy } => format!("ellipse({} {}{})", rad_s(rx), rad_s(ry), at(cx, cy)),
        Shape::Polygon { evenodd, pts } => {
            let p = pts.iter().map(|(x, y)| format!("{} {}", lp_s(*x), lp_s(*y))).collect::<Vec<_>>().join(", ");
            format!("polygon({}{})", if *evenodd { "evenodd, " } else { "" }, p)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Shapes: coverage of a point
// ---------------------------------------------------------------------------------------------

/// Whether `(px, py)` (screen px) is inside the shape for the border box `(x, y, w, h)`.
fn shape_contains(s: &Shape<LP>, (x, y, w, h): (f32, f32, f32, f32), px: f32, py: f32) -> bool {
    let rad = |r: &Rad<LP>, cx: f32, cy: f32, horiz: bool, basis: f32| -> f32 {
        match r {
            Rad::Len(l) => l.at(basis),
            Rad::Closest | Rad::Farthest => {
                let (a, b) = if horiz { ((cx - x).abs(), (x + w - cx).abs()) } else { ((cy - y).abs(), (y + h - cy).abs()) };
                if matches!(r, Rad::Closest) { a.min(b) } else { a.max(b) }
            }
        }
    };
    match s {
        Shape::Inset { t, r, b, l, round } => {
            let (x0, y0) = (x + l.at(w), y + t.at(h));
            let (x1, y1) = (x + w - r.at(w), y + h - b.at(h));
            if x1 <= x0 || y1 <= y0 {
                return false;
            }
            in_uniform_rrect(px, py, x0, y0, x1 - x0, y1 - y0, round.at(w.max(h)).max(0.0))
        }
        Shape::Circle { r, cx, cy } => {
            let (cx, cy) = (x + cx.at(w), y + cy.at(h));
            let basis = ((w * w + h * h) / 2.0).sqrt();
            let rr = match r {
                Rad::Len(l) => l.at(basis),
                Rad::Closest | Rad::Farthest => {
                    let all = [(cx - x).abs(), (x + w - cx).abs(), (cy - y).abs(), (y + h - cy).abs()];
                    if matches!(r, Rad::Closest) { all.into_iter().fold(f32::MAX, f32::min) } else { all.into_iter().fold(0.0, f32::max) }
                }
            };
            let (dx, dy) = (px - cx, py - cy);
            dx * dx + dy * dy <= rr * rr
        }
        Shape::Ellipse { rx, ry, cx, cy } => {
            let (cx, cy) = (x + cx.at(w), y + cy.at(h));
            let (rx, ry) = (rad(rx, cx, cy, true, w), rad(ry, cx, cy, false, h));
            if rx <= 0.0 || ry <= 0.0 {
                return false;
            }
            let (dx, dy) = ((px - cx) / rx, (py - cy) / ry);
            dx * dx + dy * dy <= 1.0
        }
        Shape::Polygon { evenodd, pts } => {
            if pts.len() < 3 {
                return false;
            }
            let (mut winding, mut crossings) = (0i32, 0i32);
            let pt = |k: usize| (x + pts[k].0.at(w), y + pts[k].1.at(h));
            let mut j = pts.len() - 1;
            for i in 0..pts.len() {
                let (a, b) = (pt(j), pt(i));
                if (a.1 <= py) != (b.1 <= py) {
                    let t = (py - a.1) / (b.1 - a.1);
                    if px < a.0 + t * (b.0 - a.0) {
                        crossings += 1;
                        winding += if b.1 > a.1 { 1 } else { -1 };
                    }
                }
                j = i;
            }
            if *evenodd { crossings % 2 == 1 } else { winding != 0 }
        }
    }
}

/// Coverage 0..=1 of the pixel `(ix, iy)`: 2x2 samples.
fn shape_coverage(s: &Shape<LP>, bx: (f32, f32, f32, f32), ix: i32, iy: i32) -> f32 {
    let mut hits = 0;
    for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
        if shape_contains(s, bx, ix as f32 + ox, iy as f32 + oy) {
            hits += 1;
        }
    }
    hits as f32 / 4.0
}

// ---------------------------------------------------------------------------------------------
// Pixel code
// ---------------------------------------------------------------------------------------------

/// An integer pixel rectangle `[x0, x1) x [y0, y1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Rect {
    fn w(&self) -> usize {
        (self.x1 - self.x0).max(0) as usize
    }
    fn h(&self) -> usize {
        (self.y1 - self.y0).max(0) as usize
    }
    fn area(&self) -> i64 {
        self.w() as i64 * self.h() as i64
    }
    fn intersect(&self, o: Rect) -> Rect {
        Rect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) }
    }
    fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
    fn grow(&self, m: i32) -> Rect {
        Rect { x0: self.x0 - m, y0: self.y0 - m, x1: self.x1 + m, y1: self.y1 + m }
    }
}

fn rect_of(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x0: x.floor() as i32, y0: y.floor() as i32, x1: (x + w).ceil() as i32, y1: (y + h).ceil() as i32 }
}

fn clip_rect(clip: Option<(f32, f32, f32, f32)>) -> Option<Rect> {
    clip.map(|(x, y, w, h)| rect_of(x, y, w, h))
}

/// Planar premultiplied RGBA, values 0..=1, covering `r`.
pub(crate) struct Layer {
    r: Rect,
    p: [Vec<f32>; 4],
}

impl Layer {
    fn new(r: Rect, fill: [f32; 4]) -> Layer {
        let len = r.w() * r.h();
        Layer { r, p: [vec![fill[0]; len], vec![fill[1]; len], vec![fill[2]; len], vec![fill[3]; len]] }
    }
}

fn sigma_of(blur_px: f32) -> f32 {
    blur_px.clamp(0.0, MAX_SIGMA)
}

/// How far a filter chain can move ink outwards, in px (a blur reaches about 3 sigma).
pub(crate) fn filter_reach(f: &[FilterCompFn]) -> i32 {
    let mut m = 0.0f32;
    for fun in f {
        match fun {
            FilterFn::Blur(s) => m += 3.0 * sigma_of(*s),
            FilterFn::DropShadow(s) => m += s.dx.abs().max(s.dy.abs()) + 3.0 * sigma_of(s.blur / 2.0),
            _ => {}
        }
    }
    (m.ceil() as i32).min(256)
}

/// One horizontal box pass of half-width `r` (zero outside), row by row.
fn box_h(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    let inv = 1.0 / (2 * r + 1) as f32;
    for row in 0..h {
        let (s, d) = (&src[row * w..(row + 1) * w], &mut dst[row * w..(row + 1) * w]);
        let mut acc: f32 = s[..(r + 1).min(w)].iter().sum();
        for i in 0..w {
            d[i] = acc * inv;
            if i + r + 1 < w {
                acc += s[i + r + 1];
            }
            if i >= r {
                acc -= s[i - r];
            }
        }
    }
}

/// One vertical box pass; walks whole rows so the memory access is sequential.
fn box_v(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize) {
    let inv = 1.0 / (2 * r + 1) as f32;
    let mut acc = vec![0.0f32; w];
    for row in 0..(r + 1).min(h) {
        for (a, v) in acc.iter_mut().zip(&src[row * w..(row + 1) * w]) {
            *a += *v;
        }
    }
    for i in 0..h {
        for (d, a) in dst[i * w..(i + 1) * w].iter_mut().zip(acc.iter()) {
            *d = *a * inv;
        }
        if i + r + 1 < h {
            for (a, v) in acc.iter_mut().zip(&src[(i + r + 1) * w..(i + r + 2) * w]) {
                *a += *v;
            }
        }
        if i >= r {
            for (a, v) in acc.iter_mut().zip(&src[(i - r) * w..(i - r + 1) * w]) {
                *a -= *v;
            }
        }
    }
}

/// Three box passes per axis (the classic gaussian approximation, zero outside).
fn gauss_blur(plane: &mut [f32], w: usize, h: usize, sigma: f32) {
    let r = ((12.0 * sigma * sigma / 3.0 + 1.0).sqrt().floor() as usize).max(1) / 2;
    if r == 0 || w == 0 || h == 0 {
        return;
    }
    let mut tmp = vec![0.0f32; w * h];
    for _ in 0..3 {
        box_h(plane, &mut tmp, w, h, r);
        box_v(&tmp, plane, w, h, r);
    }
}

/// Gaussian blur of one plane. A large sigma is blurred on a plane shrunk by 2, 4 or 8 (box
/// average down, bilinear back up), so the cost stays bounded whatever the radius.
fn blur_plane(plane: &mut [f32], w: usize, h: usize, sigma: f32) {
    if sigma < 0.3 || w == 0 || h == 0 {
        return;
    }
    let f = if sigma < 3.0 { 1 } else if sigma < 8.0 { 2 } else if sigma < 20.0 { 4 } else { 8 };
    if f == 1 {
        gauss_blur(plane, w, h, sigma);
        return;
    }
    let (dw, dh) = (w.div_ceil(f), h.div_ceil(f));
    let shift = f.trailing_zeros();
    let mut small = vec![0.0f32; dw * dh];
    let norm = 1.0 / (f * f) as f32;
    for y in 0..h {
        let drow = (y >> shift) * dw;
        let src = &plane[y * w..(y + 1) * w];
        for (x, v) in src.iter().enumerate() {
            small[drow + (x >> shift)] += v * norm;
        }
    }
    gauss_blur(&mut small, dw, dh, sigma / f as f32);
    let ff = f as f32;
    // Bilinear back up: the per-column indices and weights are computed once.
    let xs: Vec<(usize, usize, f32)> = (0..w)
        .map(|x| {
            let sx = ((x as f32 + 0.5) / ff - 0.5).clamp(0.0, (dw - 1) as f32);
            let x0 = sx as usize;
            (x0, (x0 + 1).min(dw - 1), sx - x0 as f32)
        })
        .collect();
    let mut row = vec![0.0f32; w];
    for y in 0..h {
        let sy = ((y as f32 + 0.5) / ff - 0.5).clamp(0.0, (dh - 1) as f32);
        let y0 = sy as usize;
        let (y1, fy) = ((y0 + 1).min(dh - 1), sy - y0 as f32);
        let (r0, r1) = (&small[y0 * dw..(y0 + 1) * dw], &small[y1 * dw..(y1 + 1) * dw]);
        for (x, o) in row.iter_mut().enumerate() {
            let (x0, x1, fx) = xs[x];
            let top = r0[x0] + (r0[x1] - r0[x0]) * fx;
            let bot = r1[x0] + (r1[x1] - r1[x0]) * fx;
            *o = top + (bot - top) * fy;
        }
        plane[y * w..(y + 1) * w].copy_from_slice(&row);
    }
}

/// The colour functions on the unpremultiplied colour. `c` is `[r, g, b]` in 0..=1.
fn color_fn(f: &FilterCompFn, c: [f32; 3]) -> [f32; 3] {
    let m3 = |m: [f32; 9]| {
        [
            m[0] * c[0] + m[1] * c[1] + m[2] * c[2],
            m[3] * c[0] + m[4] * c[1] + m[5] * c[2],
            m[6] * c[0] + m[7] * c[1] + m[8] * c[2],
        ]
    };
    match *f {
        FilterFn::Brightness(b) => [c[0] * b, c[1] * b, c[2] * b],
        FilterFn::Contrast(k) => [(c[0] - 0.5) * k + 0.5, (c[1] - 0.5) * k + 0.5, (c[2] - 0.5) * k + 0.5],
        FilterFn::Invert(a) => {
            let a = a.clamp(0.0, 1.0);
            [c[0] * (1.0 - a) + (1.0 - c[0]) * a, c[1] * (1.0 - a) + (1.0 - c[1]) * a, c[2] * (1.0 - a) + (1.0 - c[2]) * a]
        }
        FilterFn::Grayscale(a) => {
            let s = 1.0 - a.clamp(0.0, 1.0);
            m3([
                0.2126 + 0.7874 * s, 0.7152 - 0.7152 * s, 0.0722 - 0.0722 * s,
                0.2126 - 0.2126 * s, 0.7152 + 0.2848 * s, 0.0722 - 0.0722 * s,
                0.2126 - 0.2126 * s, 0.7152 - 0.7152 * s, 0.0722 + 0.9278 * s,
            ])
        }
        FilterFn::Sepia(a) => {
            let s = 1.0 - a.clamp(0.0, 1.0);
            m3([
                0.393 + 0.607 * s, 0.769 - 0.769 * s, 0.189 - 0.189 * s,
                0.349 - 0.349 * s, 0.686 + 0.314 * s, 0.168 - 0.168 * s,
                0.272 - 0.272 * s, 0.534 - 0.534 * s, 0.131 + 0.869 * s,
            ])
        }
        FilterFn::Saturate(s) => m3([
            0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s,
            0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s,
            0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s,
        ]),
        FilterFn::HueRotate(deg) => {
            let (sn, cs) = deg.to_radians().sin_cos();
            m3([
                0.213 + cs * 0.787 - sn * 0.213, 0.715 - cs * 0.715 - sn * 0.715, 0.072 - cs * 0.072 + sn * 0.928,
                0.213 - cs * 0.213 + sn * 0.143, 0.715 + cs * 0.285 + sn * 0.140, 0.072 - cs * 0.072 - sn * 0.283,
                0.213 - cs * 0.213 - sn * 0.787, 0.715 - cs * 0.715 + sn * 0.715, 0.072 + cs * 0.928 + sn * 0.072,
            ])
        }
        _ => c,
    }
}

/// Runs the chain on a premultiplied layer. The layer's region already includes the reach.
pub(crate) fn apply_filters(layer: &mut Layer, fns: &[FilterCompFn]) {
    let (w, h) = (layer.r.w(), layer.r.h());
    for f in fns {
        match f {
            FilterFn::Blur(s) => {
                let s = sigma_of(*s);
                for pl in layer.p.iter_mut() {
                    blur_plane(pl, w, h, s);
                }
            }
            FilterFn::Opacity(a) => {
                let a = a.clamp(0.0, 1.0);
                for pl in layer.p.iter_mut() {
                    for v in pl.iter_mut() {
                        *v *= a;
                    }
                }
            }
            FilterFn::DropShadow(sh) => {
                let (dx, dy) = (sh.dx.round() as i32, sh.dy.round() as i32);
                let mut a = vec![0.0f32; w * h];
                for y in 0..h as i32 {
                    let sy = y - dy;
                    if sy < 0 || sy >= h as i32 {
                        continue;
                    }
                    for x in 0..w as i32 {
                        let sx = x - dx;
                        if sx >= 0 && sx < w as i32 {
                            a[y as usize * w + x as usize] = layer.p[3][sy as usize * w + sx as usize];
                        }
                    }
                }
                blur_plane(&mut a, w, h, sigma_of(sh.blur / 2.0));
                let ca = sh.color.a() as f32 / 255.0;
                let col = [sh.color.r() as f32 / 255.0, sh.color.g() as f32 / 255.0, sh.color.b() as f32 / 255.0];
                for (i, av) in a.iter().enumerate() {
                    let sa = av * ca;
                    let inv = 1.0 - layer.p[3][i];
                    for c in 0..3 {
                        layer.p[c][i] += col[c] * sa * inv;
                    }
                    layer.p[3][i] += sa * inv;
                }
            }
            other => {
                for i in 0..w * h {
                    let a = layer.p[3][i];
                    if a <= 0.0 {
                        continue;
                    }
                    let u = [layer.p[0][i] / a, layer.p[1][i] / a, layer.p[2][i] / a];
                    let o = color_fn(other, u);
                    for c in 0..3 {
                        layer.p[c][i] = o[c].clamp(0.0, 1.0) * a;
                    }
                }
            }
        }
    }
}

fn unpack(px: u32) -> [f32; 3] {
    [((px >> 16) & 0xFF) as f32 / 255.0, ((px >> 8) & 0xFF) as f32 / 255.0, (px & 0xFF) as f32 / 255.0]
}

fn pack(c: [f32; 3]) -> u32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    q(c[0]) << 16 | q(c[1]) << 8 | q(c[2])
}

/// The separable blend functions (`cb` backdrop, `cs` source, both 0..=1).
fn blend_ch(m: BlendMode, cb: f32, cs: f32) -> f32 {
    let hard = |cb: f32, cs: f32| if cs <= 0.5 { cb * 2.0 * cs } else { let s2 = 2.0 * cs - 1.0; cb + s2 - cb * s2 };
    match m {
        BlendMode::Multiply => cb * cs,
        BlendMode::Screen => cb + cs - cb * cs,
        BlendMode::Overlay => hard(cs, cb),
        BlendMode::Darken => cb.min(cs),
        BlendMode::Lighten => cb.max(cs),
        BlendMode::ColorDodge => {
            if cb == 0.0 { 0.0 } else if cs >= 1.0 { 1.0 } else { (cb / (1.0 - cs)).min(1.0) }
        }
        BlendMode::ColorBurn => {
            if cb >= 1.0 { 1.0 } else if cs <= 0.0 { 0.0 } else { 1.0 - ((1.0 - cb) / cs).min(1.0) }
        }
        BlendMode::HardLight => hard(cb, cs),
        BlendMode::SoftLight => {
            if cs <= 0.5 {
                cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
            } else {
                let d = if cb <= 0.25 { ((16.0 * cb - 12.0) * cb + 4.0) * cb } else { cb.sqrt() };
                cb + (2.0 * cs - 1.0) * (d - cb)
            }
        }
        BlendMode::Difference => (cb - cs).abs(),
        BlendMode::Exclusion => cb + cs - 2.0 * cb * cs,
        _ => cs,
    }
}

thread_local! {
    /// Scratch pixel buffers (window sized), reused between effect boxes; taken while in use so a
    /// nested effect box allocates its own.
    static POOL: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
}

fn take_buf(len: usize) -> Vec<u32> {
    let mut v = POOL.with(|p| p.borrow_mut().pop()).unwrap_or_default();
    if v.len() != len {
        v.clear();
        v.resize(len, 0);
    }
    v
}

fn give_buf(v: Vec<u32>) {
    POOL.with(|p| {
        let mut p = p.borrow_mut();
        if p.len() < 4 {
            p.push(v);
        }
    });
}

fn fill_region(buf: &mut [u32], win_w: u32, r: Rect, v: u32) {
    for y in r.y0..r.y1 {
        let base = y as usize * win_w as usize;
        buf[base + r.x0 as usize..base + r.x1 as usize].fill(v);
    }
}

/// Which effects a style asks for.
pub(crate) fn has_effects(s: &ComputedStyle) -> bool {
    s.filter.is_some() || s.backdrop_filter.is_some() || s.clip_path.is_some() || s.mix_blend_mode != BlendMode::Normal
}

type PlainPaint = fn(&Laid, &mut FontSystem, &mut SwashCache, &mut ShapeCache, &HashMap<usize, f32>, &mut [u32], u32, u32, f32, Option<(f32, f32, f32, f32)>);

/// Paints `laid` with its `filter` / `backdrop-filter` / `clip-path` / `mix-blend-mode`. `plain`
/// paints the box itself (descendants recurse through the normal `paint`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_with_effects(
    laid: &Laid,
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
    cache: &mut ShapeCache,
    scroll_offsets: &HashMap<usize, f32>,
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    scroll_offset: f32,
    clip: Option<(f32, f32, f32, f32)>,
    plain: PlainPaint,
) {
    let s = laid.style;
    let (bx, by) = (laid.x - scroll_x(), laid.y - scroll_offset);
    let bbox = (bx, by, laid.w, laid.h);
    let win = Rect { x0: 0, y0: 0, x1: win_w as i32, y1: win_h as i32 };
    let vis = match clip_rect(clip) {
        Some(c) => win.intersect(c),
        None => win,
    };
    // backdrop-filter: filter the page pixels under the border box (rounded), then paint the box.
    if let Some(bf) = s.backdrop_filter {
        let area = rect_of(bx, by, laid.w, laid.h).intersect(vis);
        if !area.is_empty() && area.area() <= MAX_REGION_PX && s.visibility == Visibility::Visible {
            backdrop(pixels, win_w, win, area, bbox, s.border_radius.resolve(laid.w), bf);
        }
    }
    let layered = s.filter.is_some() || s.mix_blend_mode.separable();
    if !layered && s.clip_path.is_none() {
        plain(laid, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, scroll_offset, clip);
        return;
    }
    let clampf = |v: f32| v.clamp(-1.0e6, 1.0e6);
    let ink = Rect {
        x0: clampf(laid.ink.l - scroll_x()).floor() as i32,
        y0: clampf(laid.ink.t - scroll_offset).floor() as i32,
        x1: clampf(laid.ink.r - scroll_x()).ceil() as i32,
        y1: clampf(laid.ink.b - scroll_offset).ceil() as i32,
    };
    let own = rect_of(bx, by, laid.w, laid.h);
    let base = Rect { x0: ink.x0.min(own.x0), y0: ink.y0.min(own.y0), x1: ink.x1.max(own.x1), y1: ink.y1.max(own.y1) };
    let reach = s.filter.map_or(0, filter_reach);
    let region = base.grow(reach).intersect(win);
    if region.is_empty() {
        return;
    }
    if region.area() > MAX_REGION_PX {
        plain(laid, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, scroll_offset, clip);
        return;
    }
    let out = region.intersect(vis);
    let cov = |ix: i32, iy: i32| s.clip_path.map_or(1.0, |sh| shape_coverage(sh, bbox, ix, iy));
    let (rw, rh) = (region.w(), region.h());

    if !layered {
        // clip-path only: paint, then take back what the shape does not cover.
        let mut before = Vec::with_capacity(rw * rh);
        for y in region.y0..region.y1 {
            let b = (y as u32 * win_w) as usize;
            before.extend_from_slice(&pixels[b + region.x0 as usize..b + region.x1 as usize]);
        }
        plain(laid, font_system, swash_cache, cache, scroll_offsets, pixels, win_w, win_h, scroll_offset, clip);
        for y in out.y0..out.y1 {
            for x in out.x0..out.x1 {
                let m = cov(x, y);
                if m >= 1.0 {
                    continue;
                }
                let i = (y as u32 * win_w + x as u32) as usize;
                let old = unpack(before[(y - region.y0) as usize * rw + (x - region.x0) as usize]);
                let new = unpack(pixels[i]);
                pixels[i] = pack([old[0] + (new[0] - old[0]) * m, old[1] + (new[1] - old[1]) * m, old[2] + (new[2] - old[2]) * m]);
            }
        }
        return;
    }

    // Layer: paint over black and over white, recover premultiplied RGBA.
    let len = (win_w as usize) * (win_h as usize);
    let mut black = take_buf(len);
    let mut white = take_buf(len);
    fill_region(&mut black, win_w, region, 0x000000);
    fill_region(&mut white, win_w, region, 0xFFFFFF);
    let rc = Some((region.x0 as f32, region.y0 as f32, rw as f32, rh as f32));
    plain(laid, font_system, swash_cache, cache, scroll_offsets, &mut black, win_w, win_h, scroll_offset, rc);
    plain(laid, font_system, swash_cache, cache, scroll_offsets, &mut white, win_w, win_h, scroll_offset, rc);
    let mut layer = Layer::new(region, [0.0; 4]);
    for y in 0..rh {
        let row = (region.y0 as usize + y) * win_w as usize + region.x0 as usize;
        for x in 0..rw {
            let (b, w) = (black[row + x], white[row + x]);
            let d = ((w >> 16) & 0xFF) as i32 - ((b >> 16) & 0xFF) as i32
                + ((w >> 8) & 0xFF) as i32 - ((b >> 8) & 0xFF) as i32
                + (w & 0xFF) as i32 - (b & 0xFF) as i32;
            let a = (255.0 - d as f32 / 3.0).clamp(0.0, 255.0);
            if a < 1.5 {
                continue;
            }
            let i = y * rw + x;
            let bc = unpack(b);
            layer.p[0][i] = bc[0];
            layer.p[1][i] = bc[1];
            layer.p[2][i] = bc[2];
            layer.p[3][i] = a / 255.0;
        }
    }
    give_buf(black);
    give_buf(white);
    if let Some(f) = s.filter {
        apply_filters(&mut layer, f);
    }
    let mode = s.mix_blend_mode;
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let li = (y - region.y0) as usize * rw + (x - region.x0) as usize;
            let a = layer.p[3][li];
            if a <= 0.0 {
                continue;
            }
            let m = cov(x, y);
            if m <= 0.0 {
                continue;
            }
            let i = (y as u32 * win_w + x as u32) as usize;
            let dst = unpack(pixels[i]);
            let pm = [layer.p[0][li], layer.p[1][li], layer.p[2][li]];
            let aa = (a * m).min(1.0);
            let res = if !mode.separable() {
                [pm[0] * m + dst[0] * (1.0 - aa), pm[1] * m + dst[1] * (1.0 - aa), pm[2] * m + dst[2] * (1.0 - aa)]
            } else {
                let mut r = [0.0; 3];
                for c in 0..3 {
                    let cs = (pm[c] / a).min(1.0);
                    r[c] = (1.0 - aa) * dst[c] + aa * blend_ch(mode, dst[c], cs);
                }
                r
            };
            pixels[i] = pack(res);
        }
    }
}

/// `backdrop-filter`: filters the page pixels in `area` (clipped border box), writing back only
/// inside the rounded border box. The input includes the blur's reach, so the edge is not faded.
fn backdrop(pixels: &mut [u32], win_w: u32, win: Rect, area: Rect, bbox: (f32, f32, f32, f32), radius: f32, f: &[FilterCompFn]) {
    let src = area.grow(filter_reach(f)).intersect(win);
    let (rw, rh) = (src.w(), src.h());
    if rw == 0 || rh == 0 {
        return;
    }
    let mut layer = Layer::new(src, [0.0, 0.0, 0.0, 1.0]);
    for y in 0..rh {
        let row = (src.y0 as usize + y) * win_w as usize + src.x0 as usize;
        for x in 0..rw {
            let c = unpack(pixels[row + x]);
            let i = y * rw + x;
            layer.p[0][i] = c[0];
            layer.p[1][i] = c[1];
            layer.p[2][i] = c[2];
        }
    }
    apply_filters_opaque(&mut layer, f);
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            if radius > 0.0 && !in_uniform_rrect(x as f32 + 0.5, y as f32 + 0.5, bbox.0, bbox.1, bbox.2, bbox.3, radius) {
                continue;
            }
            let i = (y - src.y0) as usize * rw + (x - src.x0) as usize;
            pixels[(y as u32 * win_w + x as u32) as usize] = pack([layer.p[0][i], layer.p[1][i], layer.p[2][i]]);
        }
    }
}

/// The chain over an opaque layer (alpha stays 1). `drop-shadow()` and `opacity()` do nothing to
/// an opaque backdrop image here.
fn apply_filters_opaque(layer: &mut Layer, fns: &[FilterCompFn]) {
    let (w, h) = (layer.r.w(), layer.r.h());
    for f in fns {
        match f {
            FilterFn::Blur(s) => {
                let s = sigma_of(*s);
                // Blurring with zero outside darkens the edges: blur a coverage plane too and divide.
                let mut cov = vec![1.0f32; w * h];
                blur_plane(&mut cov, w, h, s);
                for pl in layer.p[..3].iter_mut() {
                    blur_plane(pl, w, h, s);
                    for (v, c) in pl.iter_mut().zip(cov.iter()) {
                        if *c > 0.0 {
                            *v = (*v / *c).min(1.0);
                        }
                    }
                }
            }
            FilterFn::DropShadow(_) | FilterFn::Opacity(_) => {}
            other => {
                for i in 0..w * h {
                    let u = [layer.p[0][i], layer.p[1][i], layer.p[2][i]];
                    let o = color_fn(other, u);
                    for c in 0..3 {
                        layer.p[c][i] = o[c].clamp(0.0, 1.0);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// text-shadow
// ---------------------------------------------------------------------------------------------

thread_local! {
    /// The `text-shadow` of the box being painted (inherited, so every text-bearing box carries its
    /// own); read by `paint_buffer`.
    pub(crate) static TEXT_SHADOW: Cell<Option<&'static [ShadowCompFn]>> = const { Cell::new(None) };
}

/// Draws the shadows of one shaped text run under its glyphs. `draw` replays the run's glyph walk,
/// calling its argument with each glyph pixel run `(x, y, w, h, coverage)` in screen px before the
/// shadow offset. `bounds` is the run's extent `(x, y, w, h)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_text_shadows(
    shadows: &[ShadowCompFn],
    bounds: (f32, f32, f32, f32),
    pixels: &mut [u32],
    win_w: u32,
    win_h: u32,
    clip: Option<(f32, f32, f32, f32)>,
    mut draw: impl FnMut(&mut dyn FnMut(i32, i32, u32, u32, u8)),
) {
    let win = Rect { x0: 0, y0: 0, x1: win_w as i32, y1: win_h as i32 };
    let vis = match clip_rect(clip) {
        Some(c) => win.intersect(c),
        None => win,
    };
    // The first shadow is on top, so paint the list back to front.
    for sh in shadows.iter().rev() {
        if sh.color.a() == 0 {
            continue;
        }
        let sigma = sigma_of(sh.blur / 2.0).min(16.0);
        let m = (3.0 * sigma).ceil() as i32;
        let (dx, dy) = (sh.dx.round() as i32, sh.dy.round() as i32);
        let base = rect_of(bounds.0, bounds.1, bounds.2, bounds.3).grow(2);
        let reg = Rect { x0: base.x0 + dx - m, y0: base.y0 + dy - m, x1: base.x1 + dx + m, y1: base.y1 + dy + m };
        let vis_reg = reg.intersect(vis);
        if vis_reg.is_empty() || reg.area() > MAX_REGION_PX {
            continue;
        }
        let (mw, mh) = (reg.w(), reg.h());
        let mut mask = vec![0.0f32; mw * mh];
        draw(&mut |x, y, w, h, a| {
            let v = a as f32 / 255.0;
            for yy in y..y + h as i32 {
                let ry = yy + dy - reg.y0;
                if ry < 0 || ry >= mh as i32 {
                    continue;
                }
                for xx in x..x + w as i32 {
                    let rx = xx + dx - reg.x0;
                    if rx < 0 || rx >= mw as i32 {
                        continue;
                    }
                    let cell = &mut mask[ry as usize * mw + rx as usize];
                    *cell = *cell + v - *cell * v;
                }
            }
        });
        blur_plane(&mut mask, mw, mh, sigma);
        for y in vis_reg.y0..vis_reg.y1 {
            for x in vis_reg.x0..vis_reg.x1 {
                let v = mask[(y - reg.y0) as usize * mw + (x - reg.x0) as usize];
                let a = (sh.color.a() as f32 * v).round() as u32;
                if a == 0 {
                    continue;
                }
                let i = (y as u32 * win_w + x as u32) as usize;
                pixels[i] = blend(pixels[i], TextColor::rgba(sh.color.r(), sh.color.g(), sh.color.b(), a.min(255) as u8));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Inline wrappers: the box builder flattens a non-replaced inline element (no box of its own), so
// its `filter` / `backdrop-filter` / `mix-blend-mode` would be lost. A block-level box built
// inside it (block-in-inline) takes them instead, per box. Text directly in the inline element
// is not filtered (known gap).
// ---------------------------------------------------------------------------------------------

type InlineFx = (Option<&'static [FilterCompFn]>, Option<&'static [FilterCompFn]>, BlendMode);

thread_local! {
    static INLINE_FX: Cell<Option<InlineFx>> = const { Cell::new(None) };
}

pub(crate) struct InlineFxGuard(Option<InlineFx>);

impl InlineFxGuard {
    /// Sets the ambient effects of the inline element `s` while its children are collected.
    pub(crate) fn enter(s: &ComputedStyle) -> InlineFxGuard {
        let own: InlineFx = (s.filter, s.backdrop_filter, s.mix_blend_mode);
        let prev = INLINE_FX.with(|c| c.get());
        if own.0.is_some() || own.1.is_some() || own.2 != BlendMode::Normal {
            INLINE_FX.with(|c| c.set(Some(own)));
        }
        InlineFxGuard(prev)
    }
}

impl Drop for InlineFxGuard {
    fn drop(&mut self) {
        INLINE_FX.with(|c| c.set(self.0));
    }
}

/// Runs `build` (a block box inside the inline element) with no ambient effects for its own
/// descendants, then gives the built box the effects of the inline element (unless it has its own).
pub(crate) fn with_inline_fx(build: impl FnOnce() -> LayoutBox) -> LayoutBox {
    let amb = INLINE_FX.with(|c| c.take());
    let mut b = build();
    INLINE_FX.with(|c| c.set(amb));
    if let Some((f, bf, m)) = amb {
        let s = &mut b.style;
        s.filter = s.filter.or(f);
        s.backdrop_filter = s.backdrop_filter.or(bf);
        if s.mix_blend_mode == BlendMode::Normal {
            s.mix_blend_mode = m;
        }
    }
    b
}

// ---------------------------------------------------------------------------------------------
// Animation (keyframe interpolation, called from anim_eval::lerp_styles)
// ---------------------------------------------------------------------------------------------

/// `o`'s colour at zero alpha (the identity of a shadow colour).
fn clear_of(o: TextColor) -> TextColor {
    TextColor::rgba(o.r(), o.g(), o.b(), 0)
}

fn identity_filter(f: &FilterCompFn) -> FilterCompFn {
    match f {
        FilterFn::Blur(_) => FilterFn::Blur(0.0),
        FilterFn::Brightness(_) => FilterFn::Brightness(1.0),
        FilterFn::Contrast(_) => FilterFn::Contrast(1.0),
        FilterFn::Grayscale(_) => FilterFn::Grayscale(0.0),
        FilterFn::Invert(_) => FilterFn::Invert(0.0),
        FilterFn::Opacity(_) => FilterFn::Opacity(1.0),
        FilterFn::Saturate(_) => FilterFn::Saturate(1.0),
        FilterFn::Sepia(_) => FilterFn::Sepia(0.0),
        FilterFn::HueRotate(_) => FilterFn::HueRotate(0.0),
        FilterFn::DropShadow(s) => FilterFn::DropShadow(Shadow { dx: 0.0, dy: 0.0, blur: 0.0, color: clear_of(s.color) }),
    }
}

fn lerp_shadow(a: &ShadowCompFn, b: &ShadowCompFn, t: f32) -> ShadowCompFn {
    let l = |x: f32, y: f32| x + (y - x) * t;
    let color = crate::anim_eval::Interp::interp(&a.color, &b.color, t).unwrap_or(b.color);
    Shadow { dx: l(a.dx, b.dx), dy: l(a.dy, b.dy), blur: l(a.blur, b.blur).max(0.0), color }
}

fn lerp_filter_fn(a: &FilterCompFn, b: &FilterCompFn, t: f32) -> Option<FilterCompFn> {
    let l = |x: f32, y: f32| x + (y - x) * t;
    Some(match (a, b) {
        (FilterFn::Blur(x), FilterFn::Blur(y)) => FilterFn::Blur(l(*x, *y).max(0.0)),
        (FilterFn::Brightness(x), FilterFn::Brightness(y)) => FilterFn::Brightness(l(*x, *y)),
        (FilterFn::Contrast(x), FilterFn::Contrast(y)) => FilterFn::Contrast(l(*x, *y)),
        (FilterFn::Grayscale(x), FilterFn::Grayscale(y)) => FilterFn::Grayscale(l(*x, *y)),
        (FilterFn::Invert(x), FilterFn::Invert(y)) => FilterFn::Invert(l(*x, *y)),
        (FilterFn::Opacity(x), FilterFn::Opacity(y)) => FilterFn::Opacity(l(*x, *y)),
        (FilterFn::Saturate(x), FilterFn::Saturate(y)) => FilterFn::Saturate(l(*x, *y)),
        (FilterFn::Sepia(x), FilterFn::Sepia(y)) => FilterFn::Sepia(l(*x, *y)),
        (FilterFn::HueRotate(x), FilterFn::HueRotate(y)) => FilterFn::HueRotate(l(*x, *y)),
        (FilterFn::DropShadow(x), FilterFn::DropShadow(y)) => FilterFn::DropShadow(lerp_shadow(x, y, t)),
        _ => return None,
    })
}

/// `t` snapped to 1/128 so a long animation interns at most 129 lists per keyframe pair.
fn snap_t(t: f32) -> f32 {
    (t.clamp(0.0, 1.0) * 128.0).round() / 128.0
}

/// Interpolates two `filter` / `backdrop-filter` values. `None` (outer) = the values do not
/// interpolate (different function lists): discrete. `none` pads with each other function's identity.
pub(crate) fn lerp_filter(a: Option<&'static [FilterCompFn]>, b: Option<&'static [FilterCompFn]>, t: f32) -> Option<Option<&'static [FilterCompFn]>> {
    if a.is_none() && b.is_none() {
        return Some(None);
    }
    let t = snap_t(t);
    if t <= 0.0 {
        return Some(a);
    }
    if t >= 1.0 {
        return Some(b);
    }
    let la: Vec<FilterCompFn> = match a {
        Some(l) => l.to_vec(),
        None => b?.iter().map(identity_filter).collect(),
    };
    let lb: Vec<FilterCompFn> = match b {
        Some(l) => l.to_vec(),
        None => a?.iter().map(identity_filter).collect(),
    };
    if la.len() != lb.len() {
        return None;
    }
    let out: Option<Vec<FilterCompFn>> = la.iter().zip(lb.iter()).map(|(x, y)| lerp_filter_fn(x, y, t)).collect();
    Some(Some(intern_list(&T_FILTER_COMP, out?)))
}

/// Interpolates two `text-shadow` values (the shorter list is padded with transparent shadows).
pub(crate) fn lerp_text_shadow(a: Option<&'static [ShadowCompFn]>, b: Option<&'static [ShadowCompFn]>, t: f32) -> Option<&'static [ShadowCompFn]> {
    let t = snap_t(t);
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let (la, lb) = (a.unwrap_or(&[]), b.unwrap_or(&[]));
    let zero = |o: Option<&ShadowCompFn>| Shadow { dx: 0.0, dy: 0.0, blur: 0.0, color: clear_of(o.map_or(TextColor::rgba(0, 0, 0, 0), |s| s.color)) };
    let out: Vec<ShadowCompFn> = (0..la.len().max(lb.len()))
        .map(|i| {
            let x = la.get(i).copied().unwrap_or_else(|| zero(lb.get(i)));
            let y = lb.get(i).copied().unwrap_or_else(|| zero(la.get(i)));
            lerp_shadow(&x, &y, t)
        })
        .collect();
    Some(intern_list(&T_SHADOW_COMP, out))
}
