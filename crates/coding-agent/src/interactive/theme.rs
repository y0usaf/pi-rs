//! Themes (upstream modes/interactive/theme/): the built-in dark and light themes, custom
//! theme files, and the "theme" setting with its "light/dark" automatic pair. Colors are
//! upstream's okhsl values converted by upstream's own okhslToRgb; truecolor when COLORTERM says
//! so, else the nearest xterm-256 color as upstream's rgbToAnsi256 picks it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

use pi_tui::components::editor::EditorTheme;
use pi_tui::components::markdown::{DefaultTextStyle, MarkdownTheme, StyleFn};
use pi_tui::components::select_list::SelectListTheme;

/// The theme upstream defaults to, generated from the colors the terminal reports. Here it is the
/// built-in theme of the terminal's appearance.
pub const SYSTEM_THEME: &str = "system";

const DARK_JSON: &str = include_str!("../../themes/dark.json");
const LIGHT_JSON: &str = include_str!("../../themes/light.json");

// ============================================================================
// Color
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Rgb(u8, u8, u8),
    Indexed(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Foreground,
    Background,
}

/// THEME_TOKENS: every token, the slot it is accepted in, and the token an optional one falls
/// back to.
const TOKENS: &[(&str, Slot, Option<&str>)] = &[
    ("accent", Slot::Foreground, None),
    ("border", Slot::Foreground, None),
    ("borderAccent", Slot::Foreground, None),
    ("borderMuted", Slot::Foreground, None),
    ("success", Slot::Foreground, None),
    ("error", Slot::Foreground, None),
    ("warning", Slot::Foreground, None),
    ("muted", Slot::Foreground, None),
    ("dim", Slot::Foreground, None),
    ("text", Slot::Foreground, None),
    ("thinkingText", Slot::Foreground, None),
    ("scrollbarTrack", Slot::Foreground, Some("muted")),
    ("scrollbarThumb", Slot::Foreground, Some("text")),
    ("selectedBg", Slot::Background, None),
    ("searchMatchBg", Slot::Background, Some("selectedBg")),
    ("searchMatchText", Slot::Foreground, Some("text")),
    ("userMessageBg", Slot::Background, None),
    ("userMessageText", Slot::Foreground, None),
    ("customMessageBg", Slot::Background, None),
    ("customMessageText", Slot::Foreground, None),
    ("customMessageLabel", Slot::Foreground, None),
    ("toolPendingBg", Slot::Background, None),
    ("toolSuccessBg", Slot::Background, None),
    ("toolErrorBg", Slot::Background, None),
    ("toolTitle", Slot::Foreground, None),
    ("toolOutput", Slot::Foreground, None),
    ("mdHeading", Slot::Foreground, None),
    ("mdLink", Slot::Foreground, None),
    ("mdLinkUrl", Slot::Foreground, None),
    ("mdCode", Slot::Foreground, None),
    ("mdCodeBlock", Slot::Foreground, None),
    ("mdCodeBlockBorder", Slot::Foreground, None),
    ("mdQuote", Slot::Foreground, None),
    ("mdQuoteBorder", Slot::Foreground, None),
    ("mdHr", Slot::Foreground, None),
    ("mdListBullet", Slot::Foreground, None),
    ("toolDiffAdded", Slot::Foreground, None),
    ("toolDiffRemoved", Slot::Foreground, None),
    ("toolDiffContext", Slot::Foreground, None),
    ("syntaxComment", Slot::Foreground, None),
    ("syntaxKeyword", Slot::Foreground, None),
    ("syntaxFunction", Slot::Foreground, None),
    ("syntaxVariable", Slot::Foreground, None),
    ("syntaxString", Slot::Foreground, None),
    ("syntaxNumber", Slot::Foreground, None),
    ("syntaxType", Slot::Foreground, None),
    ("syntaxOperator", Slot::Foreground, None),
    ("syntaxPunctuation", Slot::Foreground, None),
    ("thinkingOff", Slot::Foreground, None),
    ("thinkingMinimal", Slot::Foreground, None),
    ("thinkingLow", Slot::Foreground, None),
    ("thinkingMedium", Slot::Foreground, None),
    ("thinkingHigh", Slot::Foreground, None),
    ("thinkingXhigh", Slot::Foreground, None),
    ("thinkingMax", Slot::Foreground, Some("thinkingXhigh")),
    ("bashMode", Slot::Foreground, None),
];

fn token(name: &str) -> Option<(&'static str, Slot, Option<&'static str>)> {
    TOKENS.iter().find(|(token, ..)| *token == name).copied()
}

const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn truecolor() -> bool {
    static TRUECOLOR: OnceLock<bool> = OnceLock::new();
    *TRUECOLOR.get_or_init(|| {
        let ct = std::env::var("COLORTERM")
            .unwrap_or_default()
            .to_lowercase();
        ct == "truecolor" || ct == "24bit"
    })
}

fn closest(values: &[u8], v: u8) -> usize {
    let mut best = 0;
    for (i, c) in values.iter().enumerate() {
        if (*c as i32 - v as i32).abs() < (values[best] as i32 - v as i32).abs() {
            best = i;
        }
    }
    best
}

fn distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let d = |x: u8, y: u8| (x as f64 - y as f64).powi(2);
    d(a.0, b.0) * 0.299 + d(a.1, b.1) * 0.587 + d(a.2, b.2) * 0.114
}

/// Upstream rgbToAnsi256: the 6x6x6 cube, or the gray ramp for near-grays closer to it.
fn ansi256(r: u8, g: u8, b: u8) -> u8 {
    let (ri, gi, bi) = (closest(&CUBE, r), closest(&CUBE, g), closest(&CUBE, b));
    let cube = (CUBE[ri], CUBE[gi], CUBE[bi]);
    let index = 16 + 36 * ri + 6 * gi + bi;
    let grays: Vec<u8> = (0..24).map(|i| 8 + i * 10).collect();
    let gray = (0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64).round() as u8;
    let go = closest(&grays, gray);
    let gv = grays[go];
    let spread = r.max(g).max(b) - r.min(g).min(b);
    if spread < 10 && distance((r, g, b), (gv, gv, gv)) < distance((r, g, b), cube) {
        return 232 + go as u8;
    }
    index as u8
}

impl Color {
    fn rgb(self) -> (u8, u8, u8) {
        match self {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Indexed(index) => indexed_to_rgb(index),
        }
    }

    /// foregroundAnsi and backgroundAnsi.
    pub fn ansi(self, background: bool) -> String {
        let layer = if background { 48 } else { 38 };
        match self {
            Color::Indexed(index) => format!("\x1b[{layer};5;{index}m"),
            Color::Rgb(r, g, b) => {
                if truecolor() {
                    format!("\x1b[{layer};2;{r};{g};{b}m")
                } else {
                    format!("\x1b[{layer};5;{}m", ansi256(r, g, b))
                }
            }
        }
    }
}

const BASIC: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (128, 0, 0),
    (0, 128, 0),
    (128, 128, 0),
    (0, 0, 128),
    (128, 0, 128),
    (0, 128, 128),
    (192, 192, 192),
    (128, 128, 128),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (0, 0, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

fn indexed_to_rgb(index: u8) -> (u8, u8, u8) {
    match index {
        0..=15 => BASIC[index as usize],
        16..=231 => {
            let cube = index - 16;
            (
                CUBE[(cube / 36) as usize],
                CUBE[((cube % 36) / 6) as usize],
                CUBE[(cube % 6) as usize],
            )
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            (gray, gray, gray)
        }
    }
}

// ============================================================================
// Oklab and OKHSL (upstream packages/tui/src/oklab.ts)
// ============================================================================

type Vector = [f64; 3];
type Matrix = [Vector; 3];

fn multiply(m: Matrix, [x, y, z]: Vector) -> Vector {
    [
        m[0][0] * x + m[0][1] * y + m[0][2] * z,
        m[1][0] * x + m[1][1] * y + m[1][2] * z,
        m[2][0] * x + m[2][1] * y + m[2][2] * z,
    ]
}

#[allow(clippy::excessive_precision)]
const LINEAR_SRGB_TO_LMS: Matrix = [
    [0.4122214694707629, 0.5363325372617349, 0.0514459932675022],
    [0.2119034958178251, 0.6806995506452344, 0.1073969535369405],
    [0.0883024591900564, 0.2817188391361215, 0.6299787016738222],
];
#[allow(clippy::excessive_precision)]
const LMS_TO_LAB: Matrix = [
    [0.210454268309314, 0.793617774702305, -0.0040720430116193],
    [1.9779985324311684, -2.42859224204858, 0.450593709617411],
    [0.0259040424655478, 0.7827717124575296, -0.8086757549230774],
];
#[allow(clippy::excessive_precision)]
const LAB_TO_LMS: Matrix = [
    [1.0, 0.3963377773761749, 0.2158037573099136],
    [1.0, -0.1055613458156586, -0.0638541728258133],
    [1.0, -0.0894841775298119, -1.2914855480194092],
];
#[allow(clippy::excessive_precision)]
const LMS_TO_LINEAR_SRGB: Matrix = [
    [4.0767416360759583, -3.3077115392580629, 0.2309699031821043],
    [-1.2684379732850315, 2.6097573492876882, -0.341319376002657],
    [-0.0041960761386756, -0.7034186179359362, 1.7076146940746117],
];

const SATURATION_FIT: [([f64; 2], [f64; 5]); 3] = [
    (
        [-1.8817031, -0.80936501],
        [1.19086277, 1.76576728, 0.59662641, 0.75515197, 0.56771245],
    ),
    (
        [1.8144408, -1.19445267],
        [0.73956515, -0.45954404, 0.08285427, 0.12541073, -0.14503204],
    ),
    (
        [0.13110758, 1.81333971],
        [1.35733652, -0.00915799, -1.1513021, -0.50559606, 0.00692167],
    ),
];
const K1: f64 = 0.206;
const K2: f64 = 0.03;
const K3: f64 = (1.0 + K1) / (1.0 + K2);

fn okhsl_to_oklab_lightness(x: f64) -> f64 {
    (x * x + K1 * x) / (K3 * (x + K2))
}

fn linear_to_srgb(value: f64) -> f64 {
    if value > 0.0031308 {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    } else {
        12.92 * value
    }
}

fn srgb_to_linear(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn oklab_to_linear_srgb(lab: Vector) -> Vector {
    let lms = multiply(LAB_TO_LMS, lab);
    multiply(
        LMS_TO_LINEAR_SRGB,
        [lms[0].powi(3), lms[1].powi(3), lms[2].powi(3)],
    )
}

fn linear_srgb_to_rgb(linear: Vector) -> (u8, u8, u8) {
    let channel = |value: f64| (linear_to_srgb(value).clamp(0.0, 1.0) * 255.0).round() as u8;
    (channel(linear[0]), channel(linear[1]), channel(linear[2]))
}

fn rgb_to_oklab((r, g, b): (u8, u8, u8)) -> Vector {
    let rgb = [
        srgb_to_linear(r as f64 / 255.0),
        srgb_to_linear(g as f64 / 255.0),
        srgb_to_linear(b as f64 / 255.0),
    ];
    let lms = multiply(LINEAR_SRGB_TO_LMS, rgb);
    multiply(LMS_TO_LAB, [lms[0].cbrt(), lms[1].cbrt(), lms[2].cbrt()])
}

fn lms_slopes(a: f64, b: f64) -> Vector {
    [
        LAB_TO_LMS[0][1] * a + LAB_TO_LMS[0][2] * b,
        LAB_TO_LMS[1][1] * a + LAB_TO_LMS[1][2] * b,
        LAB_TO_LMS[2][1] * a + LAB_TO_LMS[2][2] * b,
    ]
}

fn max_saturation(a: f64, b: f64) -> f64 {
    let channel = SATURATION_FIT
        .iter()
        .position(|([x, y], _)| x * a + y * b > 1.0)
        .unwrap_or(2);
    let [k0, k1, k2, k3, k4] = SATURATION_FIT[channel].1;
    let weights = LMS_TO_LINEAR_SRGB[channel];
    let saturation = k0 + k1 * a + k2 * b + k3 * a * a + k4 * a * b;

    let slopes = lms_slopes(a, b);
    let base = [
        1.0 + saturation * slopes[0],
        1.0 + saturation * slopes[1],
        1.0 + saturation * slopes[2],
    ];
    let dot =
        |values: Vector| weights[0] * values[0] + weights[1] * values[1] + weights[2] * values[2];
    let f = dot([base[0].powi(3), base[1].powi(3), base[2].powi(3)]);
    let f1 = dot([
        3.0 * slopes[0] * base[0].powi(2),
        3.0 * slopes[1] * base[1].powi(2),
        3.0 * slopes[2] * base[2].powi(2),
    ]);
    let f2 = dot([
        6.0 * slopes[0].powi(2) * base[0],
        6.0 * slopes[1].powi(2) * base[1],
        6.0 * slopes[2].powi(2) * base[2],
    ]);
    saturation - (f * f1) / (f1 * f1 - 0.5 * f * f2)
}

fn cusp(a: f64, b: f64) -> (f64, f64) {
    let saturation = max_saturation(a, b);
    let linear = oklab_to_linear_srgb([1.0, saturation * a, saturation * b]);
    let lightness = 1.0 / linear[0].max(linear[1]).max(linear[2]).cbrt();
    (lightness, lightness * saturation)
}

fn max_chroma(a: f64, b: f64, lightness: f64, (cusp_l, cusp_c): (f64, f64)) -> f64 {
    if lightness <= cusp_l {
        return (cusp_c * lightness) / cusp_l;
    }
    let t = (cusp_c * (lightness - 1.0)) / (cusp_l - 1.0);
    let slopes = lms_slopes(a, b);
    let lms = [
        lightness + t * slopes[0],
        lightness + t * slopes[1],
        lightness + t * slopes[2],
    ];
    let cubes = [lms[0].powi(3), lms[1].powi(3), lms[2].powi(3)];
    let first = [
        3.0 * slopes[0] * lms[0].powi(2),
        3.0 * slopes[1] * lms[1].powi(2),
        3.0 * slopes[2] * lms[2].powi(2),
    ];
    let second = [
        6.0 * slopes[0].powi(2) * lms[0],
        6.0 * slopes[1].powi(2) * lms[1],
        6.0 * slopes[2].powi(2) * lms[2],
    ];
    let dot =
        |row: Vector, values: Vector| row[0] * values[0] + row[1] * values[1] + row[2] * values[2];
    let mut steps = f64::MAX;
    for row in LMS_TO_LINEAR_SRGB {
        let f = dot(row, cubes) - 1.0;
        let f1 = dot(row, first);
        let f2 = dot(row, second);
        let u = f1 / (f1 * f1 - 0.5 * f * f2);
        if u >= 0.0 {
            steps = steps.min(-f * u);
        }
    }
    t + steps
}

/// chromaStops: OKHSL's chroma reference points [c0, cMid, cMax] at lightness L and hue (a, b).
fn chroma_stops(l: f64, a: f64, b: f64) -> (f64, f64, f64) {
    let peak = cusp(a, b);
    let c_max = max_chroma(a, b, l, peak);
    let k = c_max / (l * (peak.1 / peak.0)).min((1.0 - l) * (peak.1 / (1.0 - peak.0)));
    let mid_s = 0.11516993
        + 1.0
            / (7.4477897
                + 4.1590124 * b
                + a * (-2.19557347
                    + 1.75198401 * b
                    + a * (-2.13704948 - 10.02301043 * b
                        + a * (-4.24894561 + 5.38770819 * b + 4.69891013 * a))));
    let mid_t = 0.11239642
        + 1.0
            / (1.6132032 - 0.68124379 * b
                + a * (0.40370612
                    + 0.90148123 * b
                    + a * (-0.27087943
                        + 0.6122399 * b
                        + a * (0.00299215 - 0.45399568 * b - 0.14661872 * a))));
    let c_mid = 0.9
        * k
        * (1.0 / (1.0 / (l * mid_s).powi(4) + 1.0 / ((1.0 - l) * mid_t).powi(4)))
            .sqrt()
            .sqrt();
    let c0 = (1.0 / (1.0 / (l * 0.4).powi(2) + 1.0 / ((1.0 - l) * 0.8).powi(2))).sqrt();
    (c0, c_mid, c_max)
}

/// okhslToRgb: OKHSL to sRGB channels, clipping out-of-gamut channels.
fn okhsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> (u8, u8, u8) {
    let l = okhsl_to_oklab_lightness(lightness);
    let mut lab: Vector = [l, 0.0, 0.0];
    if l > 0.0 && l < 1.0 && saturation > 0.0 {
        let angle = (2.0 * std::f64::consts::PI * (((hue % 360.0) + 360.0) % 360.0)) / 360.0;
        let (a, b) = (angle.cos(), angle.sin());
        let (c0, c_mid, c_max) = chroma_stops(l, a, b);
        let chroma = if saturation < 0.8 {
            let t = 1.25 * saturation;
            let k1 = 0.8 * c0;
            (t * k1) / (1.0 - (1.0 - k1 / c_mid) * t)
        } else {
            let t = 5.0 * (saturation - 0.8);
            let k1 = (0.2 * c_mid * c_mid * 1.25 * 1.25) / c0;
            c_mid + (t * k1) / (1.0 - (1.0 - k1 / (c_max - c_mid)) * t)
        };
        lab = [l, chroma * a, chroma * b];
    }
    linear_srgb_to_rgb(oklab_to_linear_srgb(lab))
}

fn is_in_srgb_gamut(linear: Vector) -> bool {
    linear
        .iter()
        .all(|channel| *channel >= -1e-7 && *channel <= 1.0 + 1e-7)
}

/// oklchToRgb: reduce chroma until the color fits sRGB, keeping its hue.
fn oklch_to_rgb(l: f64, c: f64, h: f64) -> (u8, u8, u8) {
    let radians = (h * std::f64::consts::PI) / 180.0;
    let (cos, sin) = (radians.cos(), radians.sin());
    let at_chroma = |chroma: f64| oklab_to_linear_srgb([l, chroma * cos, chroma * sin]);
    let direct = at_chroma(c);
    if is_in_srgb_gamut(direct) {
        return linear_srgb_to_rgb(direct);
    }
    let mut linear = at_chroma(0.0);
    let (mut low, mut high) = (0.0, c);
    for _ in 0..20 {
        let chroma = (low + high) / 2.0;
        let candidate = at_chroma(chroma);
        if is_in_srgb_gamut(candidate) {
            low = chroma;
            linear = candidate;
        } else {
            high = chroma;
        }
    }
    linear_srgb_to_rgb(linear)
}

// ============================================================================
// Parsing color values
// ============================================================================

/// A CSS number: [+-]?( digits[.digits] | .digits )(e[+-]digits)?
fn number(text: &str) -> Option<(f64, &str)> {
    let bytes = text.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+') | Some(b'-')) {
        end += 1;
    }
    let mut digits = 0;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
        digits += 1;
    }
    if end < bytes.len() && bytes[end] == b'.' {
        end += 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    let number_end = end;
    if end < bytes.len() && (bytes[end] == b'e' || bytes[end] == b'E') {
        let mut exponent = end + 1;
        if matches!(bytes.get(exponent), Some(b'+') | Some(b'-')) {
            exponent += 1;
        }
        let start = exponent;
        while exponent < bytes.len() && bytes[exponent].is_ascii_digit() {
            exponent += 1;
        }
        if exponent > start {
            end = exponent;
        }
    }
    text.get(..end)
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| (value, &text[number_end..]))
}

/// A number with an optional unit: % for a fraction, deg for an angle.
fn unit_number(text: &str) -> Option<(f64, Option<char>, &str)> {
    let (value, rest) = number(text)?;
    let rest = rest.trim_start();
    if let Some(rest) = rest.strip_prefix('%') {
        return Some((value, Some('%'), rest));
    }
    if let Some(rest) = rest.strip_prefix("deg") {
        return Some((value, Some('d'), rest));
    }
    Some((value, None, rest))
}

/// parseColor: a hex color, oklch(l c h) or okhsl(h s l). None is the empty string, which means
/// the terminal's own default for the token's slot.
fn parse_color(value: &str) -> Result<Option<Color>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    if let Some(hex) = value.strip_prefix('#')
        && (hex.len() == 3 || hex.len() == 6)
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        let digits: String = if hex.len() == 3 {
            hex.chars().flat_map(|c| [c, c]).collect()
        } else {
            hex.to_owned()
        };
        let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).unwrap_or(0);
        return Ok(Some(Color::Rgb(channel(0), channel(2), channel(4))));
    }

    let lower = value.to_lowercase();
    let invalid = || format!("Invalid color value: {value}");
    if let Some(rest) = lower.strip_prefix("oklch(") {
        let values = function_args(rest).ok_or_else(invalid)?;
        let l = values[0].0 / if values[0].1 == Some('%') { 100.0 } else { 1.0 };
        let c = values[1].0;
        let h = values[2].0;
        if !(0.0..=1.0).contains(&l) || c < 0.0 {
            return Err(invalid());
        }
        let (r, g, b) = oklch_to_rgb(l, c, h);
        return Ok(Some(Color::Rgb(r, g, b)));
    }
    if let Some(rest) = lower.strip_prefix("okhsl(") {
        let values = function_args(rest).ok_or_else(invalid)?;
        let h = values[0].0;
        let s = values[1].0 / if values[1].1 == Some('%') { 100.0 } else { 1.0 };
        let l = values[2].0 / if values[2].1 == Some('%') { 100.0 } else { 1.0 };
        if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&l) {
            return Err(invalid());
        }
        let (r, g, b) = okhsl_to_rgb(h, s, l);
        return Ok(Some(Color::Rgb(r, g, b)));
    }
    Err(invalid())
}

/// Three whitespace-separated numbers, each with an optional unit, then the closing parenthesis.
fn function_args(mut rest: &str) -> Option<[(f64, Option<char>); 3]> {
    let mut values = [(0.0, None); 3];
    rest = rest.trim_start();
    for value in values.iter_mut() {
        let (number, unit, tail) = unit_number(rest)?;
        *value = (number, unit);
        rest = tail.trim_start();
    }
    let tail = rest.strip_prefix(')')?;
    tail.trim().is_empty().then_some(values)
}

// ============================================================================
// Loading themes
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

impl Appearance {
    fn of(name: &str) -> Option<Appearance> {
        match name {
            "dark" => Some(Appearance::Dark),
            "light" => Some(Appearance::Light),
            _ => None,
        }
    }
}

/// A loaded theme: its tokens as the escape sequences fg() and bg() prepend.
#[derive(Debug)]
pub struct Theme {
    pub name: String,
    pub source_path: Option<String>,
    pub appearance: Appearance,
    foreground: HashMap<String, String>,
    background: HashMap<String, String>,
}

/// Where custom themes are found: the agent directory's themes/, the trusted project's .pi/themes,
/// and the paths the themes setting and --theme name.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    pub agent_dir: String,
    pub cwd: String,
    pub trusted: bool,
    pub paths: Vec<String>,
    pub no_themes: bool,
}

/// An available theme and the file it came from, when it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeInfo {
    pub name: String,
    pub path: Option<String>,
}

fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn resolve_var<'a>(
    value: &'a serde_json::Value,
    vars: &'a serde_json::Map<String, serde_json::Value>,
    visited: &mut Vec<String>,
) -> Result<&'a serde_json::Value, String> {
    let Some(text) = value.as_str() else {
        return Ok(value);
    };
    let lower = text.to_lowercase();
    if text.is_empty()
        || text.starts_with('#')
        || lower.starts_with("oklch(")
        || lower.starts_with("okhsl(")
    {
        return Ok(value);
    }
    if visited.iter().any(|seen| seen == text) {
        return Err(format!("Circular variable reference detected: {text}"));
    }
    let Some(found) = vars.get(text) else {
        return Err(format!("Variable reference not found: {text}"));
    };
    visited.push(text.to_owned());
    resolve_var(found, vars, visited)
}

/// A theme's colors with its vars resolved and its optional tokens filled from their fallbacks.
fn resolve_colors(
    colors: &serde_json::Map<String, serde_json::Value>,
    vars: &serde_json::Map<String, serde_json::Value>,
) -> Result<HashMap<String, serde_json::Value>, String> {
    let mut resolved: HashMap<String, serde_json::Value> = HashMap::new();
    for (name, value) in colors {
        if token(name).is_none() {
            return Err(format!("unknown color token {name}"));
        }
        resolved.insert(
            name.clone(),
            resolve_var(value, vars, &mut Vec::new())?.clone(),
        );
    }
    let mut missing: Vec<&str> = TOKENS
        .iter()
        .filter(|(name, _, fallback)| fallback.is_none() && !resolved.contains_key(*name))
        .map(|(name, ..)| *name)
        .collect();
    if !missing.is_empty() {
        missing.sort_unstable();
        let mut message = String::from("\nMissing required color tokens:\n");
        for name in &missing {
            message.push_str(&format!("  - {name}\n"));
        }
        message.push_str("\nPlease add these colors to your theme's \"colors\" object.");
        message.push_str("\nSee the built-in themes (dark.json, light.json) for reference values.");
        return Err(message);
    }
    let fallbacks: Vec<(String, &str)> = TOKENS
        .iter()
        .filter_map(|(name, _, fallback)| fallback.map(|fallback| ((*name).to_owned(), fallback)))
        .collect();
    for (name, fallback) in fallbacks {
        if resolved.contains_key(&name) {
            continue;
        }
        let value = resolved
            .get(fallback)
            .cloned()
            .ok_or_else(|| format!("Theme token {name} has unresolved fallback {fallback}"))?;
        resolved.insert(name, value);
    }
    Ok(resolved)
}

/// The background a theme is designed for, from the lightness of its own colors.
fn detect_appearance(colors: &HashMap<String, Option<Color>>) -> Option<Appearance> {
    let lightness = |slot: Slot| -> Option<f64> {
        let mut total = 0.0;
        let mut count = 0;
        for (name, ..) in TOKENS.iter().filter(|(_, s, _)| *s == slot) {
            let Some(Some(color)) = colors.get(*name) else {
                continue;
            };
            if matches!(color, Color::Indexed(index) if *index < 16) {
                continue;
            }
            total += rgb_to_oklab(color.rgb())[0];
            count += 1;
        }
        (count > 0).then(|| total / count as f64)
    };
    match (lightness(Slot::Foreground), lightness(Slot::Background)) {
        (Some(fg), Some(bg)) => Some(if bg < fg {
            Appearance::Dark
        } else {
            Appearance::Light
        }),
        (None, Some(bg)) => Some(if bg < 0.5 {
            Appearance::Dark
        } else {
            Appearance::Light
        }),
        (Some(fg), None) => Some(if fg > 0.5 {
            Appearance::Dark
        } else {
            Appearance::Light
        }),
        (None, None) => None,
    }
}

fn invalid_theme_name(name: &str) -> String {
    format!(
        "Invalid theme name \"{name}\": theme names cannot contain \"/\" because it is reserved for automatic light/dark theme settings."
    )
}

impl Theme {
    fn from_json(
        label: &str,
        json: &serde_json::Value,
        source_path: Option<String>,
    ) -> Result<Theme, String> {
        let Some(object) = json.as_object() else {
            return Err(format!(
                "Invalid theme \"{label}\": expected an object with a \"colors\" map."
            ));
        };
        let name = object
            .get("name")
            .and_then(|name| name.as_str())
            .ok_or_else(|| format!("Invalid theme \"{label}\": expected a name."))?;
        if name.contains('/') {
            return Err(invalid_theme_name(name));
        }
        let appearance = match object.get("appearance").and_then(|value| value.as_str()) {
            Some(value) => Appearance::of(value)
                .ok_or_else(|| format!("Invalid theme \"{label}\": unknown appearance {value}."))?,
            None => Appearance::Dark,
        };
        let vars = object
            .get("vars")
            .and_then(|vars| vars.as_object())
            .cloned()
            .unwrap_or_default();
        let colors = object
            .get("colors")
            .and_then(|colors| colors.as_object())
            .ok_or_else(|| {
                format!("Invalid theme \"{label}\": expected an object with a \"colors\" map.")
            })?;
        let resolved = resolve_colors(colors, &vars).map_err(|error| {
            if error.starts_with('\n') {
                format!("Invalid theme \"{label}\":{error}")
            } else {
                format!("Invalid theme \"{label}\": {error}")
            }
        })?;

        let mut values: HashMap<String, Option<Color>> = HashMap::new();
        for (name, value) in &resolved {
            let color = match value {
                serde_json::Value::String(text) => parse_color(text)
                    .map_err(|error| format!("Invalid theme \"{label}\": {error}"))?,
                serde_json::Value::Number(number) => match number.as_u64() {
                    Some(index) if index <= 255 => Some(Color::Indexed(index as u8)),
                    _ => {
                        return Err(format!(
                            "Invalid theme \"{label}\": color index {number} is not between 0 and 255."
                        ));
                    }
                },
                _ => {
                    return Err(format!(
                        "Invalid theme \"{label}\": color {name} is neither a string nor a palette index."
                    ));
                }
            };
            values.insert(name.clone(), color);
        }

        let mut foreground = HashMap::new();
        let mut background = HashMap::new();
        for (name, slot, _) in TOKENS {
            let escape = match values.get(*name).copied().flatten() {
                Some(color) => color.ansi(*slot == Slot::Background),
                None if *slot == Slot::Background => "\x1b[49m".to_owned(),
                None => "\x1b[39m".to_owned(),
            };
            match slot {
                Slot::Foreground => {
                    foreground.insert((*name).to_owned(), escape);
                }
                Slot::Background => {
                    background.insert((*name).to_owned(), escape);
                }
            }
        }

        Ok(Theme {
            name: name.to_owned(),
            source_path,
            appearance: object
                .get("appearance")
                .and_then(|value| value.as_str())
                .and_then(Appearance::of)
                .or_else(|| detect_appearance(&values))
                .unwrap_or(appearance),
            foreground,
            background,
        })
    }

    /// The escape sequence a token's foreground (or background) starts with.
    fn ansi(&self, name: &str, background: bool) -> Option<&str> {
        let map = if background {
            &self.background
        } else {
            &self.foreground
        };
        map.get(name).map(String::as_str)
    }
}

/// Whether the terminal is dark or light, as the COLORFGBG environment variable some terminals set
/// reports it: fg;bg or fg;xpm;bg, an ANSI palette index or "default". 0-6 and 8 are dark.
pub fn terminal_appearance() -> Appearance {
    let Some(value) = std::env::var("COLORFGBG").ok() else {
        return Appearance::Dark;
    };
    let Some(bg) = value.split(';').next_back().map(str::trim) else {
        return Appearance::Dark;
    };
    let Ok(index) = bg.parse::<u32>() else {
        return Appearance::Dark;
    };
    if index > 15 {
        return Appearance::Dark;
    }
    if index <= 6 || index == 8 {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// parseAutoThemeSetting: "light/dark" names two themes, one per terminal appearance.
pub fn parse_auto(setting: Option<&str>) -> Option<(String, String)> {
    let setting = setting?;
    let mut parts = setting.split('/');
    let light = parts.next()?.trim().to_owned();
    let dark = parts.next()?.trim().to_owned();
    if parts.next().is_some() || light.is_empty() || dark.is_empty() {
        return None;
    }
    Some((light, dark))
}

/// resolveThemeSetting: the theme a setting names for this terminal, or None when the setting names
/// a pair that does not parse.
pub fn resolve_setting(setting: Option<&str>, appearance: Appearance) -> Option<String> {
    if let Some((light, dark)) = parse_auto(setting) {
        return Some(if appearance == Appearance::Light {
            light
        } else {
            dark
        });
    }
    let setting = setting?;
    if setting.contains('/') {
        return None;
    }
    Some(setting.to_owned())
}

fn read_json(path: &str) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Failed to read theme {path}: {error}"))?;
    crate::json::parse(strip_bom(&text))
        .map_err(|error| format!("Failed to parse theme {path}: {error}"))
}

fn theme_in_dir(dir: &str, infos: &mut Vec<ThemeInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().to_string_lossy().into_owned())
        .filter(|path| path.ends_with(".json"))
        .collect();
    paths.sort();
    for path in paths {
        let Ok(json) = read_json(&path) else {
            continue;
        };
        let Ok(theme) = Theme::from_json(&path, &json, Some(path.clone())) else {
            continue;
        };
        infos.push(ThemeInfo {
            name: theme.name,
            path: Some(path),
        });
    }
}

/// getAvailableThemesWithPaths: the system theme first, then the built-in ones alphabetically, then
/// the custom themes, each name once.
pub fn available(sources: &Sources) -> Vec<ThemeInfo> {
    let mut infos = vec![
        ThemeInfo {
            name: SYSTEM_THEME.to_owned(),
            path: None,
        },
        ThemeInfo {
            name: "dark".to_owned(),
            path: None,
        },
        ThemeInfo {
            name: "light".to_owned(),
            path: None,
        },
    ];
    if !sources.no_themes {
        let mut custom = Vec::new();
        theme_in_dir(
            &crate::paths::join(&sources.agent_dir, "themes"),
            &mut custom,
        );
        if sources.trusted {
            theme_in_dir(
                &crate::paths::join(
                    &sources.cwd,
                    &format!("{}/themes", crate::config::CONFIG_DIR_NAME),
                ),
                &mut custom,
            );
        }
        for path in &sources.paths {
            let resolved =
                crate::paths::resolve_path(path, &sources.cwd, crate::paths::PathInput::default());
            if std::path::Path::new(&resolved).is_dir() {
                theme_in_dir(&resolved, &mut custom);
            } else if let Ok(json) = read_json(&resolved)
                && let Ok(theme) = Theme::from_json(&resolved, &json, Some(resolved.clone()))
            {
                custom.push(ThemeInfo {
                    name: theme.name,
                    path: Some(resolved),
                });
            }
        }
        infos.extend(custom);
    }
    let mut seen: Vec<String> = Vec::new();
    infos.retain(|info| {
        if seen.contains(&info.name) {
            false
        } else {
            seen.push(info.name.clone());
            true
        }
    });
    infos.sort_by(|a, b| {
        let key = |info: &ThemeInfo| (info.name != SYSTEM_THEME, info.name.clone());
        key(a).cmp(&key(b))
    });
    infos
}

fn builtin(appearance: Appearance) -> Result<Theme, String> {
    let (label, text) = match appearance {
        Appearance::Dark => ("dark", DARK_JSON),
        Appearance::Light => ("light", LIGHT_JSON),
    };
    let json = crate::json::parse(strip_bom(text))?;
    let mut theme = Theme::from_json(label, &json, None)?;
    theme.appearance = appearance;
    Ok(theme)
}

/// loadTheme: the system theme is the built-in theme of the terminal's appearance; anything else is
/// a built-in name or a custom theme file.
pub fn load(name: &str, sources: &Sources) -> Result<Theme, String> {
    if name == SYSTEM_THEME {
        return builtin(terminal_appearance());
    }
    if let Some(appearance) = Appearance::of(name) {
        return builtin(appearance);
    }
    let info = available(sources)
        .into_iter()
        .find(|info| info.name == name)
        .ok_or_else(|| format!("Theme not found: {name}"))?;
    let path = info
        .path
        .ok_or_else(|| format!("Theme not found: {name}"))?;
    let json = read_json(&path)?;
    let source_path = Some(path.clone());
    Theme::from_json(&path, &json, source_path)
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Theme>>> = const { RefCell::new(None) };
}

/// The theme in effect. Before init(), the built-in dark theme.
pub fn current() -> Rc<Theme> {
    CURRENT.with(|current| {
        if let Some(theme) = current.borrow().as_ref() {
            return theme.clone();
        }
        let theme = Rc::new(
            builtin(Appearance::Dark)
                .unwrap_or_else(|error| panic!("The built-in dark theme does not load: {error}")),
        );
        *current.borrow_mut() = Some(theme.clone());
        theme
    })
}

fn put(theme: Theme) {
    CURRENT.with(|current| *current.borrow_mut() = Some(Rc::new(theme)));
}

/// Apply the theme setting, falling back to the system theme when it names a theme that does not
/// load. The message it returns says why.
pub fn init(sources: &Sources, setting: Option<&str>) -> Option<String> {
    let name =
        resolve_setting(setting, terminal_appearance()).unwrap_or_else(|| SYSTEM_THEME.to_owned());
    match load(&name, sources) {
        Ok(theme) => {
            put(theme);
            None
        }
        Err(error) => {
            put(builtin(terminal_appearance()).expect("a built-in theme loads"));
            Some(format!(
                "Failed to load theme \"{name}\": {error}\nFell back to the system theme."
            ))
        }
    }
}

/// Apply a theme setting: a theme name, or a light/dark pair resolved for this terminal.
pub fn set_setting(setting: &str, sources: &Sources) -> Result<(), String> {
    let name = resolve_setting(Some(setting), terminal_appearance())
        .unwrap_or_else(|| SYSTEM_THEME.to_owned());
    set(&name, sources)
}

/// setTheme: switch to an available theme, leaving the current one in place when it does not load.
pub fn set(name: &str, sources: &Sources) -> Result<(), String> {
    let theme = load(name, sources)?;
    put(theme);
    Ok(())
}

// ============================================================================
// Drawing
// ============================================================================

fn ansi(name: &str, background: bool) -> String {
    let theme = current();
    match theme.ansi(name, background) {
        Some(escape) => escape.to_owned(),
        None => panic!("Unknown theme color: {name}"),
    }
}

/// theme.fg(token, text).
pub fn fg(name: &str, text: &str) -> String {
    format!("{}{text}\x1b[39m", ansi(name, false))
}

/// theme.bg(token, text).
pub fn bg(name: &str, text: &str) -> String {
    format!("{}{text}\x1b[49m", ansi(name, true))
}

pub fn bold(text: &str) -> String {
    format!("\x1b[1m{text}\x1b[22m")
}

pub fn italic(text: &str) -> String {
    format!("\x1b[3m{text}\x1b[23m")
}

fn underline(text: &str) -> String {
    format!("\x1b[4m{text}\x1b[24m")
}

pub fn strikethrough(text: &str) -> String {
    format!("\x1b[9m{text}\x1b[29m")
}

/// A style function drawing a token in the foreground.
pub fn fg_fn(name: &'static str) -> StyleFn {
    Rc::new(move |text: &str| fg(name, text))
}

pub fn bg_fn(name: &'static str) -> StyleFn {
    Rc::new(move |text: &str| bg(name, text))
}

/// getMarkdownTheme(), without syntax highlighting yet: code blocks take mdCodeBlock, as upstream
/// draws a block whose language it does not know.
pub fn markdown() -> MarkdownTheme {
    MarkdownTheme {
        heading: fg_fn("mdHeading"),
        link: fg_fn("mdLink"),
        link_url: fg_fn("mdLinkUrl"),
        code: fg_fn("mdCode"),
        code_block: fg_fn("mdCodeBlock"),
        code_block_border: fg_fn("mdCodeBlockBorder"),
        quote: fg_fn("mdQuote"),
        quote_border: fg_fn("mdQuoteBorder"),
        hr: fg_fn("mdHr"),
        list_bullet: fg_fn("mdListBullet"),
        bold: Rc::new(bold),
        italic: Rc::new(italic),
        strikethrough: Rc::new(strikethrough),
        underline: Rc::new(underline),
        highlight_code: Some(Rc::new(|code: &str, lang: Option<&str>| {
            match lang.filter(|lang| super::syntax::supports(lang)) {
                Some(lang) => super::syntax::highlight(code, lang),
                None => code.split('\n').map(|l| fg("mdCodeBlock", l)).collect(),
            }
        })),
        code_block_indent: None,
    }
}

/// The user message's text color and background.
pub fn user_style() -> DefaultTextStyle {
    DefaultTextStyle {
        color: Some(fg_fn("userMessageText")),
        bg_color: Some(bg_fn("userMessageBg")),
        ..DefaultTextStyle::default()
    }
}

/// Thinking blocks: thinkingText, italic.
pub fn thinking_style() -> DefaultTextStyle {
    DefaultTextStyle {
        color: Some(fg_fn("thinkingText")),
        italic: true,
        ..DefaultTextStyle::default()
    }
}

/// getEditorTheme().
pub fn editor() -> EditorTheme {
    EditorTheme {
        border_color: fg_fn("borderMuted"),
        select_list: SelectListTheme {
            selected_prefix: fg_fn("accent"),
            selected_text: fg_fn("accent"),
            description: fg_fn("muted"),
            scroll_info: fg_fn("muted"),
            no_match: fg_fn("muted"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi256_matches_upstream_cube_and_grays() {
        assert_eq!(ansi256(0, 0, 0), 16);
        assert_eq!(ansi256(255, 255, 255), 231);
        assert_eq!(ansi256(128, 128, 128), 244);
    }

    #[test]
    fn okhsl_matches_the_colors_upstream_computed() {
        // The dark theme's rendered colors, as upstream's okhslToRgb produced them.
        assert_eq!(okhsl_to_rgb(295.0, 0.50, 0.67), (167, 152, 215));
        assert_eq!(okhsl_to_rgb(232.0, 0.54, 0.67), (105, 173, 208));
        assert_eq!(okhsl_to_rgb(234.0, 0.03, 0.89), (222, 224, 225));
        assert_eq!(okhsl_to_rgb(233.0, 0.41, 0.24), (33, 59, 73));
    }
}
