use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Rgb(Rgb),
    Indexed(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Ansi256,
    TrueColor,
}

pub fn true_color(env: &dyn Fn(&str) -> Option<String>) -> bool {
    let var = |name: &str| env(name).unwrap_or_default().to_lowercase();
    let set = |name: &str| env(name).is_some_and(|value| !value.is_empty());
    let (program, emulator, term, color_term) = (
        var("TERM_PROGRAM"),
        var("TERMINAL_EMULATOR"),
        var("TERM"),
        var("COLORTERM"),
    );
    let hint = color_term == "truecolor" || color_term == "24bit" || term.ends_with("-direct");
    let detected = if set("TMUX")
        || term.starts_with("tmux")
        || term.starts_with("screen")
        || program == "herdr"
    {
        hint
    } else {
        program == "rex"
            || set("KITTY_WINDOW_ID")
            || program == "kitty"
            || program == "ghostty"
            || term.contains("ghostty")
            || set("GHOSTTY_RESOURCES_DIR")
            || set("WEZTERM_PANE")
            || program == "wezterm"
            || program == "warpterminal"
            || set("WARP_SESSION_ID")
            || set("WARP_TERMINAL_SESSION_UUID")
            || set("ITERM_SESSION_ID")
            || program == "iterm.app"
            || set("WT_SESSION")
            || program == "alacritty"
            || program == "vscode"
            || program == "zed"
            || emulator == "jetbrains-jediterm"
            || cfg!(windows)
            || hint
    };
    match env("PI_TRUE_COLOR").as_deref() {
        Some("1") => true,
        Some("0") => false,
        _ => detected,
    }
}

pub fn terminal_color_mode() -> ColorMode {
    static MODE: OnceLock<ColorMode> = OnceLock::new();
    *MODE.get_or_init(|| {
        if true_color(&|name| std::env::var(name).ok()) {
            ColorMode::TrueColor
        } else {
            ColorMode::Ansi256
        }
    })
}

const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn closest(values: &[u8], v: u8) -> usize {
    let mut best = 0;
    for (i, c) in values.iter().enumerate() {
        if (*c as i32 - v as i32).abs() < (values[best] as i32 - v as i32).abs() {
            best = i;
        }
    }
    best
}

fn distance(a: Rgb, b: Rgb) -> f64 {
    let d = |x: u8, y: u8| (x as f64 - y as f64).powi(2);
    d(a.r, b.r) * 0.299 + d(a.g, b.g) * 0.587 + d(a.b, b.b) * 0.114
}

pub fn rgb_to_ansi256(color: Rgb) -> u8 {
    let Rgb { r, g, b } = color;
    let (ri, gi, bi) = (closest(&CUBE, r), closest(&CUBE, g), closest(&CUBE, b));
    let cube = Rgb::new(CUBE[ri], CUBE[gi], CUBE[bi]);
    let index = 16 + 36 * ri + 6 * gi + bi;
    let grays: Vec<u8> = (0..24).map(|i| 8 + i * 10).collect();
    let gray = (0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64).round() as u8;
    let go = closest(&grays, gray);
    let gv = grays[go];
    let spread = r.max(g).max(b) - r.min(g).min(b);
    if spread < 10 && distance(color, Rgb::new(gv, gv, gv)) < distance(color, cube) {
        return 232 + go as u8;
    }
    index as u8
}

const BASIC: [Rgb; 16] = [
    Rgb::new(0, 0, 0),
    Rgb::new(128, 0, 0),
    Rgb::new(0, 128, 0),
    Rgb::new(128, 128, 0),
    Rgb::new(0, 0, 128),
    Rgb::new(128, 0, 128),
    Rgb::new(0, 128, 128),
    Rgb::new(192, 192, 192),
    Rgb::new(128, 128, 128),
    Rgb::new(255, 0, 0),
    Rgb::new(0, 255, 0),
    Rgb::new(255, 255, 0),
    Rgb::new(0, 0, 255),
    Rgb::new(255, 0, 255),
    Rgb::new(0, 255, 255),
    Rgb::new(255, 255, 255),
];

fn indexed_to_rgb(index: u8) -> Rgb {
    match index {
        0..=15 => BASIC[index as usize],
        16..=231 => {
            let cube = index - 16;
            Rgb::new(
                CUBE[(cube / 36) as usize],
                CUBE[((cube % 36) / 6) as usize],
                CUBE[(cube % 6) as usize],
            )
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            Rgb::new(gray, gray, gray)
        }
    }
}

pub fn color_to_rgb(color: Color) -> Rgb {
    match color {
        Color::Rgb(rgb) => rgb,
        Color::Indexed(index) => indexed_to_rgb(index),
    }
}

fn color_ansi(color: Color, mode: ColorMode, background: bool) -> String {
    let layer = if background { 48 } else { 38 };
    match (color, mode) {
        (Color::Indexed(index), _) => format!("\x1b[{layer};5;{index}m"),
        (Color::Rgb(Rgb { r, g, b }), ColorMode::TrueColor) => {
            format!("\x1b[{layer};2;{r};{g};{b}m")
        }
        (Color::Rgb(rgb), ColorMode::Ansi256) => {
            format!("\x1b[{layer};5;{}m", rgb_to_ansi256(rgb))
        }
    }
}

pub fn foreground_ansi(color: Color, mode: ColorMode) -> String {
    color_ansi(color, mode, false)
}

pub fn background_ansi(color: Color, mode: ColorMode) -> String {
    color_ansi(color, mode, true)
}

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

pub fn oklab_to_okhsl_lightness(x: f64) -> f64 {
    0.5 * (K3 * x - K1 + ((K3 * x - K1).powi(2) + 4.0 * K2 * K3 * x).sqrt())
}

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

fn hypot(a: f64, b: f64) -> f64 {
    let (a, b) = (a.abs(), b.abs());
    let max = if b > a { b } else { a };
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for value in [a, b] {
        let n = value / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

fn oklab_to_linear_srgb(lab: Vector) -> Vector {
    let lms = multiply(LAB_TO_LMS, lab);
    multiply(
        LMS_TO_LINEAR_SRGB,
        [lms[0].powf(3.0), lms[1].powf(3.0), lms[2].powf(3.0)],
    )
}

fn linear_srgb_to_rgb(linear: Vector) -> Rgb {
    let channel = |value: f64| (linear_to_srgb(value).clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb::new(channel(linear[0]), channel(linear[1]), channel(linear[2]))
}

pub fn rgb_to_oklab(Rgb { r, g, b }: Rgb) -> Vector {
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
    let f = dot([base[0].powf(3.0), base[1].powf(3.0), base[2].powf(3.0)]);
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
    let lightness = (1.0 / linear[0].max(linear[1]).max(linear[2])).cbrt();
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
    let cubes = [lms[0].powf(3.0), lms[1].powf(3.0), lms[2].powf(3.0)];
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
        * (1.0 / (1.0 / (l * mid_s).powf(4.0) + 1.0 / ((1.0 - l) * mid_t).powf(4.0)))
            .sqrt()
            .sqrt();
    let c0 = (1.0 / (1.0 / (l * 0.4).powi(2) + 1.0 / ((1.0 - l) * 0.8).powi(2))).sqrt();
    (c0, c_mid, c_max)
}

pub fn okhsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> Rgb {
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
            let k1 = (0.2 * c_mid.powi(2) * 1.5625) / c0;
            c_mid + (t * k1) / (1.0 - (1.0 - k1 / (c_max - c_mid)) * t)
        };
        lab = [l, chroma * a, chroma * b];
    }
    linear_srgb_to_rgb(oklab_to_linear_srgb(lab))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Okhsl {
    pub h: f64,
    pub s: f64,
    pub l: f64,
}

pub fn rgb_to_okhsl(rgb: Rgb) -> Okhsl {
    let [big_l, lab_a, lab_b] = rgb_to_oklab(rgb);
    let chroma = hypot(lab_a, lab_b);
    let lightness = oklab_to_okhsl_lightness(big_l);
    if chroma < 1e-9 || lightness <= 0.0 || lightness >= 1.0 {
        return Okhsl {
            h: 0.0,
            s: 0.0,
            l: lightness,
        };
    }
    let hue = ((lab_b.atan2(lab_a) * 180.0) / std::f64::consts::PI + 360.0) % 360.0;
    let (c0, c_mid, c_max) = chroma_stops(big_l, lab_a / chroma, lab_b / chroma);
    let saturation = if chroma < c_mid {
        let k1 = 0.8 * c0;
        0.8 * (chroma / (k1 + (1.0 - k1 / c_mid) * chroma))
    } else {
        let k1 = (0.2 * c_mid.powi(2) * 1.5625) / c0;
        let offset = chroma - c_mid;
        0.8 + 0.2 * (offset / (k1 + (1.0 - k1 / (c_max - c_mid)) * offset))
    };
    Okhsl {
        h: hue,
        s: saturation.clamp(0.0, 1.0),
        l: lightness,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

pub fn rgb_to_oklch(rgb: Rgb) -> Oklch {
    let [l, a, b] = rgb_to_oklab(rgb);
    Oklch {
        l,
        c: hypot(a, b),
        h: ((b.atan2(a) * 180.0) / std::f64::consts::PI + 360.0) % 360.0,
    }
}

fn is_in_srgb_gamut(linear: Vector) -> bool {
    linear
        .iter()
        .all(|channel| *channel >= -1e-7 && *channel <= 1.0 + 1e-7)
}

pub fn oklch_to_rgb(l: f64, c: f64, h: f64) -> Rgb {
    let h = ((h % 360.0) + 360.0) % 360.0;
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
