//! QCNV3: CSS Color 4/5 beyond what cssparser-color gives us.
//!
//! `cssparser_color::Color::parse` already reads `lab()`, `lch()`, `oklab()`, `oklch()` and `color()`; what it does not
//! have is the conversion of those to sRGB bytes, `color-mix()` and relative colours (`rgb(from red g r b)`). This
//! module adds both: [`parse`] is a drop-in for `Color::parse` that also accepts `color-mix()` and relative colours
//! (they resolve at parse time to a `Lab`/`Oklab`/`ColorFunction` value; `currentcolor` inside them needs the
//! `current` argument, which the stylesheet path does not have, so it is rejected there), and [`to_srgb8`] converts any
//! parsed colour to sRGB bytes (out-of-gamut values are clamped).
//!
//! Colour maths follows the CSS Color 4 sample code (matrices, D50 adaptation, Lab/Oklab). Everything is `f64` inside.

use cssparser::{ParseError, Parser, Token};
use cssparser_color::{Color, ColorFunction, Hsl, Hwb, Lab, Lch, Oklab, Oklch};
use cssparser::color::PredefinedColorSpace as Pcs;

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Space {
    Srgb,
    SrgbLinear,
    P3,
    P3Linear,
    A98,
    ProPhoto,
    Rec2020,
    Xyz50,
    Xyz65,
    Lab,
    Lch,
    Oklab,
    Oklch,
    Hsl,
    Hwb,
}

impl Space {
    fn polar(self) -> Option<usize> {
        match self {
            Space::Lch | Space::Oklch => Some(2),
            Space::Hsl | Space::Hwb => Some(0),
            _ => None,
        }
    }
}

/// A colour with possibly-missing (`none`) components. Hsl/Hwb: hue in degrees, the others as fractions 0..1.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Col {
    pub space: Space,
    pub c: [Option<f64>; 3],
    pub a: Option<f64>,
}

type M = [[f64; 3]; 3];

fn mul(m: &M, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

const SRGB_TO_XYZ: M = [[0.41239079926595934, 0.357584339383878, 0.1804807884018343], [0.21263900587151027, 0.715168678767756, 0.07219231536073371], [0.01933081871559182, 0.11919477979462598, 0.9505321522496607]];
const XYZ_TO_SRGB: M = [[3.2409699419045226, -1.537383177570094, -0.4986107602930034], [-0.9692436362808796, 1.8759675015077202, 0.04155505740717559], [0.05563007969699366, -0.20397695888897652, 1.0569715142428786]];
const P3_TO_XYZ: M = [[0.4865709486482162, 0.26566769316909306, 0.1982172852343625], [0.2289745640697488, 0.6917385218365064, 0.079286914093745], [0.0, 0.04511338185890264, 1.043944368900976]];
const XYZ_TO_P3: M = [[2.493496911941425, -0.9313836179191239, -0.40271078445071684], [-0.8294889695615747, 1.7626640603183463, 0.023624685841943577], [0.03584583024378447, -0.07617238926804182, 0.9568845240076872]];
const A98_TO_XYZ: M = [[0.5766690429101305, 0.1855582379065463, 0.1882286462349947], [0.29734497525053605, 0.6273635662554661, 0.0752914584939978], [0.02703136138641234, 0.07068885253582723, 0.9913375368376388]];
const XYZ_TO_A98: M = [[2.0415879038107465, -0.5650069742788596, -0.34473135077832956], [-0.9692436362808795, 1.8759675015077202, 0.04155505740717557], [0.013444280632031142, -0.11836239223101838, 1.0151749943912054]];
const R2020_TO_XYZ: M = [[0.6369580483012914, 0.14461690358620832, 0.1688809751641721], [0.2627002120112671, 0.6779980715188708, 0.05930171646986196], [0.0, 0.028072693049087428, 1.060985057710791]];
const XYZ_TO_R2020: M = [[1.7166511879712674, -0.35567078377639233, -0.25336628137365974], [-0.6666843518324892, 1.6164812366349395, 0.01576854581391113], [0.017639857445310783, -0.042770613257808524, 0.9421031212354738]];
const PRO_TO_XYZ50: M = [[0.7977604896723027, 0.13518583717574031, 0.0313493495815248], [0.2880711282292934, 0.7118432178101014, 0.00008565396060525902], [0.0, 0.0, 0.8251046025104601]];
const XYZ50_TO_PRO: M = [[1.3457989731028281, -0.25558010007997534, -0.05110628506753401], [-0.5446224939028347, 1.5082327413132781, 0.02053603239147973], [0.0, 0.0, 1.2119675456389454]];
const D65_TO_D50: M = [[1.0479298208405488, 0.022946793341019088, -0.05019222954313557], [0.029627815688159344, 0.990434484573249, -0.01707382502938514], [-0.009243058152591178, 0.015055144896577895, 0.7518742899580008]];
const D50_TO_D65: M = [[0.9554734527042182, -0.023098536874261423, 0.0632593086610217], [-0.028369706963208136, 1.0099954580058226, 0.021041398966943008], [0.012314001688319899, -0.020507696433477912, 1.3303659366080753]];
const XYZ_TO_LMS: M = [[0.819022437996703, 0.3619062600528904, -0.1288737815209879], [0.0329836539323885, 0.9292868615863434, 0.0361446663506424], [0.0481771893596242, 0.2642395317527308, 0.6335478284694309]];
const LMS_TO_XYZ: M = [[1.2268798758459243, -0.5578149944602171, 0.2813910456659647], [-0.0405757452148008, 1.112286803280317, -0.0717110580655164], [-0.0763729366746601, -0.4214933324022432, 1.5869240198367816]];
const LMS_TO_OKLAB: M = [[0.210454268309314, 0.7936177747023054, -0.0040720430116193], [1.9779985324311684, -2.42859224204858, 0.450593709617411], [0.0259040424655478, 0.7827717124575296, -0.8086757549230774]];
const OKLAB_TO_LMS: M = [[1.0, 0.3963377773761749, 0.2158037573099136], [1.0, -0.1055613458156586, -0.0638541728258133], [1.0, -0.0894841775298119, -1.2914855480194092]];
const D50_WHITE: [f64; 3] = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];

fn srgb_to_lin(c: f64) -> f64 {
    let a = c.abs();
    if a <= 0.04045 { c / 12.92 } else { c.signum() * ((a + 0.055) / 1.055).powf(2.4) }
}
fn srgb_from_lin(c: f64) -> f64 {
    let a = c.abs();
    if a <= 0.0031308 { c * 12.92 } else { c.signum() * (1.055 * a.powf(1.0 / 2.4) - 0.055) }
}
fn pow_to_lin(c: f64, g: f64) -> f64 {
    c.signum() * c.abs().powf(g)
}
fn pow_from_lin(c: f64, g: f64) -> f64 {
    c.signum() * c.abs().powf(1.0 / g)
}
fn r2020_to_lin(c: f64) -> f64 {
    let (alpha, beta) = (1.09929682680944, 0.018053968510807);
    let a = c.abs();
    if a < beta * 4.5 { c / 4.5 } else { c.signum() * ((a + alpha - 1.0) / alpha).powf(1.0 / 0.45) }
}
fn r2020_from_lin(c: f64) -> f64 {
    let (alpha, beta) = (1.09929682680944, 0.018053968510807);
    let a = c.abs();
    if a > beta { c.signum() * (alpha * a.powf(0.45) - (alpha - 1.0)) } else { 4.5 * c }
}
fn pro_to_lin(c: f64) -> f64 {
    let a = c.abs();
    if a <= 16.0 / 512.0 { c / 16.0 } else { c.signum() * a.powf(1.8) }
}
fn pro_from_lin(c: f64) -> f64 {
    let a = c.abs();
    if a >= 1.0 / 512.0 { c.signum() * a.powf(1.0 / 1.8) } else { 16.0 * c }
}

fn hsl_to_srgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let h = h.rem_euclid(360.0);
    let f = |n: f64| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        l - s * l.min(1.0 - l) * (k - 3.0).min(9.0 - k).min(1.0).max(-1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}
fn srgb_to_hsl(rgb: [f64; 3]) -> [f64; 3] {
    let [r, g, b] = rgb;
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let (mut h, mut s, l) = (0.0, 0.0, (min + max) / 2.0);
    let d = max - min;
    if d != 0.0 {
        s = if l == 0.0 || l == 1.0 { 0.0 } else { (max - l) / l.min(1.0 - l) };
        h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        h *= 60.0;
    }
    if s < 0.0 {
        h += 180.0;
        s = s.abs();
    }
    [h.rem_euclid(360.0), s, l]
}
fn hwb_to_srgb(h: f64, w: f64, b: f64) -> [f64; 3] {
    if w + b >= 1.0 {
        let g = w / (w + b);
        return [g, g, g];
    }
    let rgb = hsl_to_srgb(h, 1.0, 0.5);
    [rgb[0] * (1.0 - w - b) + w, rgb[1] * (1.0 - w - b) + w, rgb[2] * (1.0 - w - b) + w]
}
fn srgb_to_hwb(rgb: [f64; 3]) -> [f64; 3] {
    let hsl = srgb_to_hsl(rgb);
    let (w, b) = (rgb[0].min(rgb[1]).min(rgb[2]), 1.0 - rgb[0].max(rgb[1]).max(rgb[2]));
    [hsl[0], w, b]
}

fn xyz_to_lab(xyz: [f64; 3]) -> [f64; 3] {
    let (eps, kappa) = (216.0 / 24389.0, 24389.0 / 27.0);
    let f: Vec<f64> = (0..3)
        .map(|i| {
            let t = xyz[i] / D50_WHITE[i];
            if t > eps { t.cbrt() } else { (kappa * t + 16.0) / 116.0 }
        })
        .collect();
    [116.0 * f[1] - 16.0, 500.0 * (f[0] - f[1]), 200.0 * (f[1] - f[2])]
}
fn lab_to_xyz(lab: [f64; 3]) -> [f64; 3] {
    let (eps, kappa) = (216.0 / 24389.0, 24389.0 / 27.0);
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = lab[1] / 500.0 + fy;
    let fz = fy - lab[2] / 200.0;
    let cube = |f: f64| if f * f * f > eps { f * f * f } else { (116.0 * f - 16.0) / kappa };
    let y = if lab[0] > kappa * eps { fy * fy * fy } else { lab[0] / kappa };
    [cube(fx) * D50_WHITE[0], y * D50_WHITE[1], cube(fz) * D50_WHITE[2]]
}
fn xyz65_to_oklab(xyz: [f64; 3]) -> [f64; 3] {
    let lms = mul(&XYZ_TO_LMS, xyz);
    mul(&LMS_TO_OKLAB, [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()])
}
fn oklab_to_xyz65(lab: [f64; 3]) -> [f64; 3] {
    let lms = mul(&OKLAB_TO_LMS, lab);
    mul(&LMS_TO_XYZ, [lms[0].powi(3), lms[1].powi(3), lms[2].powi(3)])
}
fn lch_of(lab: [f64; 3]) -> [f64; 3] {
    let c = lab[1].hypot(lab[2]);
    let h = if c < 1e-9 { 0.0 } else { lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.0) };
    [lab[0], c, h]
}
fn lab_of(lch: [f64; 3]) -> [f64; 3] {
    let h = lch[2].to_radians();
    [lch[0], lch[1] * h.cos(), lch[1] * h.sin()]
}

fn z(v: Option<f64>) -> f64 {
    v.unwrap_or(0.0)
}

/// The colour as CIE XYZ with D65 white (the hub every conversion goes through).
fn to_xyz65(col: &Col) -> [f64; 3] {
    let c = [z(col.c[0]), z(col.c[1]), z(col.c[2])];
    let rgb = |m: &M, f: fn(f64) -> f64| mul(m, [f(c[0]), f(c[1]), f(c[2])]);
    match col.space {
        Space::Srgb => rgb(&SRGB_TO_XYZ, srgb_to_lin),
        Space::SrgbLinear => mul(&SRGB_TO_XYZ, c),
        Space::P3 => rgb(&P3_TO_XYZ, srgb_to_lin),
        Space::P3Linear => mul(&P3_TO_XYZ, c),
        Space::A98 => rgb(&A98_TO_XYZ, |v| pow_to_lin(v, 563.0 / 256.0)),
        Space::Rec2020 => rgb(&R2020_TO_XYZ, r2020_to_lin),
        Space::ProPhoto => mul(&D50_TO_D65, mul(&PRO_TO_XYZ50, [pro_to_lin(c[0]), pro_to_lin(c[1]), pro_to_lin(c[2])])),
        Space::Xyz50 => mul(&D50_TO_D65, c),
        Space::Xyz65 => c,
        Space::Lab => mul(&D50_TO_D65, lab_to_xyz(c)),
        Space::Lch => mul(&D50_TO_D65, lab_to_xyz(lab_of(c))),
        Space::Oklab => oklab_to_xyz65(c),
        Space::Oklch => oklab_to_xyz65(lab_of(c)),
        Space::Hsl => {
            let s = hsl_to_srgb(c[0], c[1], c[2]);
            mul(&SRGB_TO_XYZ, [srgb_to_lin(s[0]), srgb_to_lin(s[1]), srgb_to_lin(s[2])])
        }
        Space::Hwb => {
            let s = hwb_to_srgb(c[0], c[1], c[2]);
            mul(&SRGB_TO_XYZ, [srgb_to_lin(s[0]), srgb_to_lin(s[1]), srgb_to_lin(s[2])])
        }
    }
}

fn from_xyz65(space: Space, x: [f64; 3]) -> [f64; 3] {
    let rgb = |m: &M, f: fn(f64) -> f64| {
        let l = mul(m, x);
        [f(l[0]), f(l[1]), f(l[2])]
    };
    match space {
        Space::Srgb => rgb(&XYZ_TO_SRGB, srgb_from_lin),
        Space::SrgbLinear => mul(&XYZ_TO_SRGB, x),
        Space::P3 => rgb(&XYZ_TO_P3, srgb_from_lin),
        Space::P3Linear => mul(&XYZ_TO_P3, x),
        Space::A98 => rgb(&XYZ_TO_A98, |v| pow_from_lin(v, 563.0 / 256.0)),
        Space::Rec2020 => rgb(&XYZ_TO_R2020, r2020_from_lin),
        Space::ProPhoto => {
            let l = mul(&XYZ50_TO_PRO, mul(&D65_TO_D50, x));
            [pro_from_lin(l[0]), pro_from_lin(l[1]), pro_from_lin(l[2])]
        }
        Space::Xyz50 => mul(&D65_TO_D50, x),
        Space::Xyz65 => x,
        Space::Lab => xyz_to_lab(mul(&D65_TO_D50, x)),
        Space::Lch => lch_of(xyz_to_lab(mul(&D65_TO_D50, x))),
        Space::Oklab => xyz65_to_oklab(x),
        Space::Oklch => lch_of(xyz65_to_oklab(x)),
        Space::Hsl => {
            let l = mul(&XYZ_TO_SRGB, x);
            srgb_to_hsl([srgb_from_lin(l[0]), srgb_from_lin(l[1]), srgb_from_lin(l[2])])
        }
        Space::Hwb => {
            let l = mul(&XYZ_TO_SRGB, x);
            srgb_to_hwb([srgb_from_lin(l[0]), srgb_from_lin(l[1]), srgb_from_lin(l[2])])
        }
    }
}

/// `col` in `space` (missing components are kept when the space does not change).
pub(crate) fn convert(col: &Col, space: Space) -> Col {
    if col.space == space {
        return *col;
    }
    let c = from_xyz65(space, to_xyz65(col));
    // Powerless polar components stay missing: a grey has no hue.
    Col { space, c: [Some(c[0]), Some(c[1]), Some(c[2])], a: col.a }
}

fn f(v: Option<f32>) -> Option<f64> {
    v.map(f64::from)
}

/// The cssparser value as a [`Col`]; `currentcolor` is `None`.
pub(crate) fn from_css(c: &Color) -> Option<Col> {
    Some(match c {
        Color::CurrentColor => return None,
        Color::Rgba(r) => Col { space: Space::Srgb, c: [Some(f64::from(r.red) / 255.0), Some(f64::from(r.green) / 255.0), Some(f64::from(r.blue) / 255.0)], a: Some(f64::from(r.alpha)) },
        Color::Hsl(h) => Col { space: Space::Hsl, c: [f(h.hue), f(h.saturation), f(h.lightness)], a: f(h.alpha) },
        Color::Hwb(h) => Col { space: Space::Hwb, c: [f(h.hue), f(h.whiteness), f(h.blackness)], a: f(h.alpha) },
        Color::Lab(l) => Col { space: Space::Lab, c: [f(l.lightness), f(l.a), f(l.b)], a: f(l.alpha) },
        Color::Lch(l) => Col { space: Space::Lch, c: [f(l.lightness), f(l.chroma), f(l.hue)], a: f(l.alpha) },
        Color::Oklab(l) => Col { space: Space::Oklab, c: [f(l.lightness), f(l.a), f(l.b)], a: f(l.alpha) },
        Color::Oklch(l) => Col { space: Space::Oklch, c: [f(l.lightness), f(l.chroma), f(l.hue)], a: f(l.alpha) },
        Color::ColorFunction(cf) => {
            let space = match cf.color_space {
                Pcs::Srgb => Space::Srgb,
                Pcs::SrgbLinear => Space::SrgbLinear,
                Pcs::DisplayP3 => Space::P3,
                Pcs::DisplayP3Linear => Space::P3Linear,
                Pcs::A98Rgb => Space::A98,
                Pcs::ProphotoRgb => Space::ProPhoto,
                Pcs::Rec2020 => Space::Rec2020,
                Pcs::XyzD50 => Space::Xyz50,
                Pcs::XyzD65 => Space::Xyz65,
            };
            Col { space, c: [f(cf.c1), f(cf.c2), f(cf.c3)], a: f(cf.alpha) }
        }
    })
}

/// The parsed value of `col` (hsl/hwb become `color(srgb ...)`).
pub(crate) fn to_css(col: &Col) -> Color {
    let col = if matches!(col.space, Space::Hsl | Space::Hwb) { convert(col, Space::Srgb) } else { *col };
    let [a, b, c] = col.c.map(|v| v.map(|x| x as f32));
    let al = col.a.map(|x| x as f32);
    let func = |s| Color::ColorFunction(ColorFunction::new(s, a, b, c, al));
    match col.space {
        Space::Srgb => func(Pcs::Srgb),
        Space::SrgbLinear => func(Pcs::SrgbLinear),
        Space::P3 => func(Pcs::DisplayP3),
        Space::P3Linear => func(Pcs::DisplayP3Linear),
        Space::A98 => func(Pcs::A98Rgb),
        Space::ProPhoto => func(Pcs::ProphotoRgb),
        Space::Rec2020 => func(Pcs::Rec2020),
        Space::Xyz50 => func(Pcs::XyzD50),
        Space::Xyz65 => func(Pcs::XyzD65),
        Space::Lab => Color::Lab(Lab { lightness: a, a: b, b: c, alpha: al }),
        Space::Lch => Color::Lch(Lch { lightness: a, chroma: b, hue: c, alpha: al }),
        Space::Oklab => Color::Oklab(Oklab { lightness: a, a: b, b: c, alpha: al }),
        Space::Oklch => Color::Oklch(Oklch { lightness: a, chroma: b, hue: c, alpha: al }),
        Space::Hsl => Color::Hsl(Hsl { hue: a, saturation: b, lightness: c, alpha: al }),
        Space::Hwb => Color::Hwb(Hwb { hue: a, whiteness: b, blackness: c, alpha: al }),
    }
}

/// Straight sRGB bytes of any parsed colour (clamped to the gamut); `currentcolor` is `None`.
pub(crate) fn to_srgb8(c: &Color) -> Option<[u8; 4]> {
    let col = from_css(c)?;
    let s = convert(&col, Space::Srgb);
    let byte = |v: Option<f64>| (z(v).clamp(0.0, 1.0) * 255.0).round() as u8;
    Some([byte(s.c[0]), byte(s.c[1]), byte(s.c[2]), (col.a.unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).round() as u8])
}

// ------------------------------------------------------------ parsing

type PResult<T> = Result<T, ParseError<()>>;

fn space_of(name: &str) -> Option<Space> {
    Some(match name.to_ascii_lowercase().as_str() {
        "srgb" => Space::Srgb,
        "srgb-linear" => Space::SrgbLinear,
        "display-p3" => Space::P3,
        "display-p3-linear" => Space::P3Linear,
        "a98-rgb" => Space::A98,
        "prophoto-rgb" => Space::ProPhoto,
        "rec2020" => Space::Rec2020,
        "xyz" | "xyz-d65" => Space::Xyz65,
        "xyz-d50" => Space::Xyz50,
        "lab" => Space::Lab,
        "lch" => Space::Lch,
        "oklab" => Space::Oklab,
        "oklch" => Space::Oklch,
        "hsl" => Space::Hsl,
        "hwb" => Space::Hwb,
        _ => return None,
    })
}

/// Parses a colour, also accepting `color-mix()` and relative colours. `current` is what `currentcolor` means inside
/// them (and as a bare keyword); without it a bare `currentcolor` stays `Color::CurrentColor` and inside a mix or a
/// relative colour it is an error.
pub(crate) fn parse<'i>(input: &mut Parser<'i>, current: Option<&Color>) -> PResult<Color> {
    let r = input.try_parse(|i| -> PResult<Color> {
        let name = i.expect_function()?.clone();
        let lower = name.to_ascii_lowercase();
        if lower == "color-mix" {
            return i.parse_nested_block(|b| mix(b, current));
        }
        if matches!(lower.as_str(), "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklab" | "oklch" | "color") {
            return i.parse_nested_block(|b| {
                b.expect_ident_matching("from").map_err(|e| ParseError::from(e))?;
                relative(b, &lower, current)
            });
        }
        Err(ParseError::unexpected_token())
    });
    if let Ok(c) = r {
        return Ok(c);
    }
    let c = Color::parse(input)?;
    Ok(match (c, current) {
        (Color::CurrentColor, Some(cur)) => *cur,
        (c, _) => c,
    })
}

fn nested_color<'i>(input: &mut Parser<'i>, current: Option<&Color>) -> PResult<Color> {
    let c = parse(input, current)?;
    if matches!(c, Color::CurrentColor) {
        return Err(ParseError::unexpected_token());
    }
    Ok(c)
}

fn mix<'i>(input: &mut Parser<'i>, current: Option<&Color>) -> PResult<Color> {
    input.expect_ident_matching("in").map_err(ParseError::from)?;
    let sp = input.expect_ident()?.clone();
    let Some(space) = space_of(&sp) else { return Err(ParseError::unexpected_token()) };
    // [shorter | longer | increasing | decreasing] hue
    let mut method = "shorter".to_string();
    if let Ok(m) = input.try_parse(|i| -> PResult<String> {
        let m = i.expect_ident()?.to_ascii_lowercase();
        i.expect_ident_matching("hue").map_err(ParseError::from)?;
        Ok(m)
    }) {
        if space.polar().is_none() || !matches!(m.as_str(), "shorter" | "longer" | "increasing" | "decreasing") {
            return Err(ParseError::unexpected_token());
        }
        method = m;
    }
    input.expect_comma()?;
    let item = |i: &mut Parser<'i>| -> PResult<(Color, Option<f64>)> {
        let mut pct = i.try_parse(|i| i.expect_percentage()).ok().map(f64::from);
        let c = nested_color(i, current)?;
        if pct.is_none() {
            pct = i.try_parse(|i| i.expect_percentage()).ok().map(f64::from);
        }
        if pct.is_some_and(|p| !(0.0..=1.0).contains(&p)) {
            return Err(ParseError::unexpected_token());
        }
        Ok((c, pct))
    };
    let (c1, p1) = item(input)?;
    input.expect_comma()?;
    let (c2, p2) = item(input)?;
    input.expect_exhausted()?;
    let (mut p1, mut p2) = match (p1, p2) {
        (None, None) => (0.5, 0.5),
        (Some(a), None) => (a, 1.0 - a),
        (None, Some(b)) => (1.0 - b, b),
        (Some(a), Some(b)) => (a, b),
    };
    let sum = p1 + p2;
    if sum == 0.0 {
        return Err(ParseError::unexpected_token());
    }
    let mult = sum.min(1.0);
    p1 /= sum;
    p2 /= sum;
    let (Some(a), Some(b)) = (from_css(&c1), from_css(&c2)) else { return Err(ParseError::unexpected_token()) };
    Ok(to_css(&mix_cols(&a, &b, p1, p2, space, &method, mult)))
}

fn mix_cols(a: &Col, b: &Col, p1: f64, p2: f64, space: Space, method: &str, mult: f64) -> Col {
    let (a, b) = (convert(a, space), convert(b, space));
    // Missing components take the other colour's value.
    let pick = |x: Option<f64>, y: Option<f64>| (x.or(y), y.or(x));
    let (a1, a2) = pick(a.a, b.a);
    let (al1, al2) = (a1.unwrap_or(1.0), a2.unwrap_or(1.0));
    let alpha = al1 * p1 + al2 * p2;
    let hue = space.polar();
    let mut out = [None; 3];
    for i in 0..3 {
        let (x, y) = pick(a.c[i], b.c[i]);
        out[i] = match (x, y) {
            (Some(mut x), Some(mut y)) => Some(if hue == Some(i) {
                let d = y - x;
                match method {
                    "longer" => {
                        if d > 0.0 && d < 180.0 { x += 360.0 } else if d > -180.0 && d <= 0.0 { y += 360.0 }
                    }
                    "increasing" => {
                        if d < 0.0 { y += 360.0 }
                    }
                    "decreasing" => {
                        if d > 0.0 { x += 360.0 }
                    }
                    _ => {
                        if d > 180.0 { x += 360.0 } else if d < -180.0 { y += 360.0 }
                    }
                }
                (x * p1 + y * p2).rem_euclid(360.0)
            } else if alpha == 0.0 {
                x * p1 + y * p2
            } else {
                (x * al1 * p1 + y * al2 * p2) / alpha
            }),
            _ => None,
        };
    }
    let alpha = if a1.is_none() && a2.is_none() { None } else { Some((alpha * mult).clamp(0.0, 1.0)) };
    Col { space, c: out, a: alpha }
}

/// Channel layout of a relative-colour function: keyword names and the percentage reference of each channel.
struct Layout {
    space: Space,
    names: [&'static str; 3],
    /// 100% of channel i, in the written units.
    refs: [f64; 3],
    /// Written units -> `Col` units.
    scale: [f64; 3],
}

fn layout(func: &str, space: Option<Space>) -> Option<Layout> {
    Some(match func {
        "rgb" | "rgba" => Layout { space: Space::Srgb, names: ["r", "g", "b"], refs: [255.0; 3], scale: [1.0 / 255.0; 3] },
        "hsl" | "hsla" => Layout { space: Space::Hsl, names: ["h", "s", "l"], refs: [360.0, 100.0, 100.0], scale: [1.0, 0.01, 0.01] },
        "hwb" => Layout { space: Space::Hwb, names: ["h", "w", "b"], refs: [360.0, 100.0, 100.0], scale: [1.0, 0.01, 0.01] },
        "lab" => Layout { space: Space::Lab, names: ["l", "a", "b"], refs: [100.0, 125.0, 125.0], scale: [1.0; 3] },
        "lch" => Layout { space: Space::Lch, names: ["l", "c", "h"], refs: [100.0, 150.0, 360.0], scale: [1.0; 3] },
        "oklab" => Layout { space: Space::Oklab, names: ["l", "a", "b"], refs: [1.0, 0.4, 0.4], scale: [1.0; 3] },
        "oklch" => Layout { space: Space::Oklch, names: ["l", "c", "h"], refs: [1.0, 0.4, 360.0], scale: [1.0; 3] },
        "color" => {
            let space = space?;
            let names = match space {
                Space::Xyz50 | Space::Xyz65 => ["x", "y", "z"],
                _ => ["r", "g", "b"],
            };
            Layout { space, names, refs: [1.0; 3], scale: [1.0; 3] }
        }
        _ => return None,
    })
}

fn relative<'i>(input: &mut Parser<'i>, func: &str, current: Option<&Color>) -> PResult<Color> {
    let origin = nested_color(input, current)?;
    let Some(origin) = from_css(&origin) else { return Err(ParseError::unexpected_token()) };
    let space = if func == "color" {
        let id = input.expect_ident()?.clone();
        match space_of(&id) {
            Some(s) if !matches!(s, Space::Lab | Space::Lch | Space::Oklab | Space::Oklch | Space::Hsl | Space::Hwb) => Some(s),
            _ => return Err(ParseError::unexpected_token()),
        }
    } else {
        None
    };
    let Some(lay) = layout(func, space) else { return Err(ParseError::unexpected_token()) };
    let o = convert(&origin, lay.space);
    // The origin's channels in the written units; `none` reads as 0.
    let vals: Vec<f64> = (0..3).map(|i| z(o.c[i]) / lay.scale[i]).collect();
    let alpha_v = o.a.unwrap_or(0.0);
    let mut out: [Option<f64>; 3] = [None; 3];
    for i in 0..3 {
        let tok = input.next()?.clone();
        out[i] = channel(&tok, Some((&lay, &vals)), lay.refs[i], &lay.names)?.map(|v| v * lay.scale[i]);
    }
    let mut alpha = Some(alpha_v);
    // `/ alpha`
    if input.try_parse(|i| i.expect_delim('/')).is_ok() {
        let tok = input.next()?.clone();
        alpha = channel(&tok, Some((&lay, &vals)), 1.0, &["alpha"])?.map(|a| a.clamp(0.0, 1.0));
        if let Token::Ident(id) = &tok
            && id.eq_ignore_ascii_case("alpha")
        {
            alpha = Some(alpha_v);
        }
    }
    input.expect_exhausted()?;
    Ok(to_css(&Col { space: lay.space, c: out, a: alpha }))
}

/// One channel of a relative colour: a keyword of the origin, `none`, a number or a percentage. `alpha` as the keyword
/// of the alpha slot is resolved by the caller (the layout names do not hold it), so here it reads 1 and is replaced.
fn channel(tok: &Token<'_>, lookup: Option<(&Layout, &[f64])>, pct_ref: f64, names: &[&str]) -> PResult<Option<f64>> {
    match tok {
        Token::Number { value, .. } => Ok(Some(f64::from(*value))),
        Token::Percentage { unit_value, .. } => Ok(Some(f64::from(*unit_value) * pct_ref)),
        Token::Ident(id) => {
            if id.eq_ignore_ascii_case("none") {
                return Ok(None);
            }
            if names == ["alpha"] {
                // alpha slot: `alpha` itself is handled by the caller; channel keywords read as their value
                if id.eq_ignore_ascii_case("alpha") {
                    return Ok(Some(1.0));
                }
            }
            if let Some((lay, vals)) = lookup {
                for (i, n) in lay.names.iter().enumerate() {
                    if id.eq_ignore_ascii_case(n) {
                        return Ok(Some(vals[i]));
                    }
                }
            }
            Err(ParseError::unexpected_token())
        }
        _ => Err(ParseError::unexpected_token()),
    }
}

/// A CSS system colour keyword (case-insensitive) as sRGB, light scheme (the values of the common engine defaults).
pub(crate) fn system_color(name: &str) -> Option<[u8; 3]> {
    Some(match name.to_ascii_lowercase().as_str() {
        "accentcolor" | "highlight" | "selecteditem" => [0x00, 0x75, 0xff],
        "accentcolortext" | "highlighttext" | "selecteditemtext" | "canvas" | "field" | "window" | "buttonhighlight" | "threedhighlight" => [0xff, 0xff, 0xff],
        "activetext" => [0xff, 0x00, 0x00],
        "buttonborder" => [0x76, 0x76, 0x76],
        "buttonface" | "threedface" | "menu" => [0xef, 0xef, 0xef],
        "buttontext" | "canvastext" | "fieldtext" | "windowtext" | "captiontext" | "infotext" | "menutext" | "marktext" => [0, 0, 0],
        "graytext" => [0x80, 0x80, 0x80],
        "linktext" => [0x00, 0x00, 0xee],
        "visitedtext" => [0x55, 0x1a, 0x8b],
        "mark" => [0xff, 0xff, 0x00],
        "activeborder" | "inactiveborder" | "windowframe" => [0xc0, 0xc0, 0xc0],
        "activecaption" | "inactivecaption" => [0x99, 0x99, 0x99],
        "appworkspace" => [0xab, 0xab, 0xab],
        "background" => [0x63, 0x63, 0xce],
        "buttonshadow" | "threedshadow" => [0xb0, 0xb0, 0xb0],
        "inactivecaptiontext" => [0x43, 0x5a, 0x73],
        "infobackground" => [0xfb, 0xfc, 0xc5],
        "scrollbar" => [0xc8, 0xc8, 0xc8],
        "threeddarkshadow" => [0x69, 0x69, 0x69],
        "threedlightshadow" => [0xe3, 0xe3, 0xe3],
        _ => return None,
    })
}
