use std::collections::HashMap;

use pi_tui::colors::{
    Color, Okhsl, Rgb, okhsl_to_rgb, oklab_to_okhsl_lightness, oklch_to_rgb, rgb_to_okhsl,
    rgb_to_oklch,
};

use super::Appearance;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Family {
    hue: f64,
    min: f64,
    max: f64,
    slot: usize,
}

const fn family(hue: f64, min: f64, max: f64, slot: usize) -> Family {
    Family {
        hue,
        min,
        max,
        slot,
    }
}

const NEUTRAL: Family = family(231.49, 0.02, 0.08, 8);
const BLUE: Family = family(231.49, 0.1, 0.68, 4);
const GREEN: Family = family(158.68, 0.1, 0.76, 2);
const RED: Family = family(20.0, 0.1, 0.92, 1);
const YELLOW: Family = family(82.36, 0.5, 1.0, 3);
const ORANGE: Family = family(52.0, 0.12, 0.85, 3);
const VIOLET: Family = family(295.0, 0.2, 0.6, 5);
const CALAMINE: Family = family(202.43, 0.1, 0.74, 6);
const THINKING_SLATE: Family = family(231.49, 0.08, 0.2, 4);
const THINKING_BLUE: Family = family(231.49, 0.2, 0.45, 4);
const THINKING_PERIWINKLE: Family = family(263.25, 0.3, 0.6, 6);
const THINKING_VIOLET: Family = family(295.0, 0.4, 0.75, 5);
const THINKING_MAGENTA: Family = family(337.5, 0.5, 0.85, 13);
const THINKING_RED: Family = family(20.0, 0.95, 1.0, 1);

const TOKEN_FAMILIES: [(&str, Family); 56] = [
    ("selectedBg", BLUE),
    ("searchMatchBg", ORANGE),
    ("userMessageBg", BLUE),
    ("customMessageBg", VIOLET),
    ("toolPendingBg", NEUTRAL),
    ("toolSuccessBg", GREEN),
    ("toolErrorBg", RED),
    ("text", NEUTRAL),
    ("userMessageText", NEUTRAL),
    ("customMessageText", NEUTRAL),
    ("toolTitle", NEUTRAL),
    ("syntaxOperator", NEUTRAL),
    ("syntaxPunctuation", NEUTRAL),
    ("muted", NEUTRAL),
    ("dim", NEUTRAL),
    ("thinkingText", NEUTRAL),
    ("toolOutput", NEUTRAL),
    ("mdLinkUrl", NEUTRAL),
    ("mdQuote", NEUTRAL),
    ("mdQuoteBorder", NEUTRAL),
    ("mdHr", NEUTRAL),
    ("mdCodeBlockBorder", NEUTRAL),
    ("toolDiffContext", NEUTRAL),
    ("syntaxComment", NEUTRAL),
    ("scrollbarTrack", NEUTRAL),
    ("scrollbarThumb", NEUTRAL),
    ("searchMatchText", NEUTRAL),
    ("borderMuted", NEUTRAL),
    ("accent", VIOLET),
    ("borderAccent", VIOLET),
    ("customMessageLabel", VIOLET),
    ("mdCode", VIOLET),
    ("mdListBullet", VIOLET),
    ("syntaxType", VIOLET),
    ("border", BLUE),
    ("mdLink", BLUE),
    ("syntaxKeyword", BLUE),
    ("syntaxVariable", CALAMINE),
    ("success", GREEN),
    ("mdCodeBlock", GREEN),
    ("toolDiffAdded", GREEN),
    ("bashMode", GREEN),
    ("syntaxNumber", GREEN),
    ("error", RED),
    ("toolDiffRemoved", RED),
    ("warning", YELLOW),
    ("mdHeading", YELLOW),
    ("syntaxFunction", YELLOW),
    ("syntaxString", ORANGE),
    ("thinkingOff", NEUTRAL),
    ("thinkingMinimal", THINKING_SLATE),
    ("thinkingLow", THINKING_BLUE),
    ("thinkingMedium", THINKING_PERIWINKLE),
    ("thinkingHigh", THINKING_VIOLET),
    ("thinkingXhigh", THINKING_MAGENTA),
    ("thinkingMax", THINKING_RED),
];

fn family_of(token: &str) -> Family {
    TOKEN_FAMILIES
        .into_iter()
        .find(|(name, _)| *name == token)
        .map_or_else(
            || panic!("no color family for {token}"),
            |(_, family)| family,
        )
}

fn slot_of(token: &str) -> usize {
    match token {
        "syntaxString" => 2,
        "syntaxNumber" => 5,
        "searchMatchBg" => 3,
        _ => family_of(token).slot,
    }
}

struct Curve {
    coefficients: [f64; 6],
    reachable: (f64, f64),
}

const fn curve(coefficients: [f64; 6], low: f64, high: f64) -> Curve {
    Curve {
        coefficients,
        reachable: (low, high),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Panel,
    Track,
    Thinking0,
    Thinking1,
    Thinking2,
    Thinking3,
    Thinking4,
    Thinking5,
    Thinking6,
    Subtle,
    Thumb,
    Readable,
    Emphasis,
    TextOnPanel,
    Text,
}

fn level_curve(level: Level, appearance: Appearance) -> Curve {
    let dark = appearance == Appearance::Dark;
    match (level, dark) {
        (Level::Panel, true) => curve(
            [0.29131, -0.39746, 2.33185, -0.85524, -1.2076, 0.86276],
            0.0,
            0.979,
        ),
        (Level::Panel, false) => curve(
            [-3.74073, 27.94549, -78.44258, 112.6798, -79.60015, 22.11277],
            0.348,
            1.0,
        ),
        (Level::Track, true) => curve(
            [0.39028, -0.23015, 0.83573, 2.43829, -4.38292, 2.01582],
            0.0,
            0.946,
        ),
        (Level::Track, false) => curve(
            [
                -5.24921, 38.37322, -107.28833, 152.10005, -106.17127, 29.18061,
            ],
            0.368,
            1.0,
        ),
        (Level::Thinking0, true) => curve(
            [0.52988, -0.05809, -0.30924, 4.63567, -6.52933, 2.89108],
            0.0,
            0.873,
        ),
        (Level::Thinking0, false) => curve(
            [
                -28.27749, 182.85284, -469.62416, 603.15916, -384.59976, 97.35147,
            ],
            0.51,
            1.0,
        ),
        (Level::Thinking1, true) => curve(
            [0.55278, -0.03667, -0.45659, 4.95347, -6.90265, 3.0706],
            0.0,
            0.858,
        ),
        (Level::Thinking1, false) => curve(
            [
                -37.10484, 235.86282, -596.62344, 754.3633, -474.00763, 118.3551,
            ],
            0.535,
            1.0,
        ),
        (Level::Thinking2, true) => curve(
            [0.57486, -0.01765, -0.58987, 5.25227, -7.27175, 3.25532],
            0.0,
            0.842,
        ),
        (Level::Thinking2, false) => curve(
            [
                -59.89653, 377.05024, -945.07843, 1182.03145, -734.96375, 181.68658,
            ],
            0.556,
            1.0,
        ),
        (Level::Thinking3, true) => curve(
            [0.59621, -0.00062, -0.71148, 5.53588, -7.6392, 3.44606],
            0.0,
            0.827,
        ),
        (Level::Thinking3, false) => curve(
            [
                -72.07122,
                445.84082,
                -1099.57352,
                1353.88793,
                -829.53392,
                202.26164,
            ],
            0.58,
            1.0,
        ),
        (Level::Thinking4, true) => curve(
            [0.61691, 0.01462, -0.82288, 5.80651, -8.00641, 3.64333],
            0.0,
            0.811,
        ),
        (Level::Thinking4, false) => curve(
            [
                -110.14338,
                674.21488,
                -1645.75941,
                2004.32367,
                -1215.15899,
                293.3183,
            ],
            0.6,
            1.0,
        ),
        (Level::Thinking5, true) => curve(
            [0.63702, 0.02826, -0.92498, 6.06465, -8.37246, 3.84651],
            0.0,
            0.795,
        ),
        (Level::Thinking5, false) => curve(
            [
                -175.47701,
                1063.54495,
                -2570.70594,
                3098.80776,
                -1860.15527,
                444.76392,
            ],
            0.62,
            1.0,
        ),
        (Level::Thinking6, true) => curve(
            [0.65658, 0.04044, -1.01835, 6.30989, -8.73529, 4.05439],
            0.0,
            0.779,
        ),
        (Level::Thinking6, false) => curve(
            [
                -183.81712,
                1094.70055,
                -2602.68539,
                3088.71276,
                -1826.91131,
                430.75931,
            ],
            0.643,
            1.0,
        ),
        (Level::Subtle, true) => curve(
            [0.56762, -0.02475, -0.5383, 5.12628, -7.10931, 3.17324],
            0.0,
            0.848,
        ),
        (Level::Subtle, false) => curve(
            [
                -232.85459,
                1376.54473,
                -3249.11801,
                3827.91186,
                -2248.29472,
                526.55751,
            ],
            0.657,
            1.0,
        ),
        (Level::Thumb, true) => curve(
            [0.60323, 0.00278, -0.73328, 5.57157, -7.68067, 3.46933],
            0.0,
            0.823,
        ),
        (Level::Thumb, false) => curve(
            [
                -82.89897,
                511.01355,
                -1255.98095,
                1540.76821,
                -940.68087,
                228.58523,
            ],
            0.586,
            1.0,
        ),
        (Level::Readable, true) => curve(
            [0.66937, 0.04704, -1.06871, 6.43941, -8.9332, 4.17229],
            0.0,
            0.77,
        ),
        (Level::Readable, false) => curve(
            [
                -1554.52576,
                8733.56817,
                -19604.93507,
                21977.72696,
                -12300.99599,
                2749.81288,
            ],
            0.751,
            1.0,
        ),
        (Level::Emphasis, true) => curve(
            [0.7303, 0.07695, -1.31626, 7.1681, -10.14436, 4.92846],
            0.0,
            0.712,
        ),
        (Level::Emphasis, false) => curve(
            [
                -4948.31942,
                26870.91986,
                -58334.48399,
                63280.17197,
                -34298.01053,
                7430.30146,
            ],
            0.811,
            1.0,
        ),
        (Level::TextOnPanel, true) => curve(
            [0.86713, 0.05232, -0.89428, 4.79014, -5.5432, 1.75023],
            0.0,
            0.542,
        ),
        (Level::TextOnPanel, false) => curve(
            [
                -8570.89457,
                43954.60805,
                -90084.00702,
                92220.6791,
                -47152.15802,
                9632.27113,
            ],
            0.867,
            1.0,
        ),
        (Level::Text, true) => curve(
            [0.89242, 0.02311, -0.44862, 2.34417, -0.06084, -2.63844],
            0.0,
            0.5,
        ),
        (Level::Text, false) => curve(
            [
                -2004.67048,
                6664.47299,
                -6060.70202,
                -1792.61209,
                5133.82359,
                -1939.85583,
            ],
            0.894,
            1.0,
        ),
    }
}

struct Rule {
    token: &'static str,
    on: Vec<&'static str>,
    level: Level,
}

const TOOL_PANELS: [&str; 3] = ["toolPendingBg", "toolSuccessBg", "toolErrorBg"];
const MESSAGE_PANELS: [&str; 2] = ["userMessageBg", "customMessageBg"];
const PANELS: [&str; 7] = [
    "userMessageBg",
    "toolPendingBg",
    "toolSuccessBg",
    "toolErrorBg",
    "selectedBg",
    "searchMatchBg",
    "customMessageBg",
];
const THINKING: [(&str, Level); 7] = [
    ("thinkingOff", Level::Thinking0),
    ("thinkingMinimal", Level::Thinking1),
    ("thinkingLow", Level::Thinking2),
    ("thinkingMedium", Level::Thinking3),
    ("thinkingHigh", Level::Thinking4),
    ("thinkingXhigh", Level::Thinking5),
    ("thinkingMax", Level::Thinking6),
];
const FOREGROUND_TOKENS: [&str; 3] = ["text", "userMessageText", "toolTitle"];
const TEXT_MINIMUM_WCAG_CONTRAST: f64 = 4.5;

fn surfaces(groups: &[&[&'static str]]) -> Vec<&'static str> {
    groups.concat()
}

fn rules() -> Vec<Rule> {
    let mut rules = Vec::new();
    let mut each = |tokens: &[&'static str], on: Vec<&'static str>, level: Level| {
        for token in tokens {
            rules.push(Rule {
                token,
                on: on.clone(),
                level,
            });
        }
    };
    each(&PANELS, vec!["background"], Level::Panel);
    each(&["text"], vec!["background"], Level::Text);
    each(&["text"], vec!["selectedBg"], Level::TextOnPanel);
    each(
        &["userMessageText"],
        vec!["userMessageBg"],
        Level::TextOnPanel,
    );
    each(&["toolTitle"], TOOL_PANELS.to_vec(), Level::TextOnPanel);
    each(
        &["accent", "success", "error", "warning"],
        surfaces(&[&["background", "selectedBg"], &TOOL_PANELS]),
        Level::Readable,
    );
    let muted = surfaces(&[
        &["background", "selectedBg", "customMessageBg"],
        &TOOL_PANELS,
    ]);
    each(&["muted"], muted.clone(), Level::Readable);
    each(&["dim"], muted, Level::Subtle);
    each(&["thinkingText"], vec!["background"], Level::Readable);
    each(
        &["customMessageText"],
        surfaces(&[&["customMessageBg"], &TOOL_PANELS]),
        Level::Readable,
    );
    each(
        &["customMessageLabel"],
        surfaces(&[
            &["background", "customMessageBg", "selectedBg"],
            &TOOL_PANELS,
        ]),
        Level::Readable,
    );
    let tool = surfaces(&[&["background"], &TOOL_PANELS]);
    each(&["toolOutput"], tool.clone(), Level::Readable);
    each(
        &[
            "mdHeading",
            "mdLink",
            "mdLinkUrl",
            "mdCode",
            "mdQuote",
            "mdCodeBlockBorder",
            "mdListBullet",
        ],
        surfaces(&[&["background"], &MESSAGE_PANELS]),
        Level::Readable,
    );
    let everywhere = surfaces(&[&["background"], &MESSAGE_PANELS, &TOOL_PANELS]);
    each(&["mdCodeBlock"], everywhere.clone(), Level::Readable);
    each(
        &["toolDiffAdded", "toolDiffRemoved", "toolDiffContext"],
        tool,
        Level::Readable,
    );
    each(
        &[
            "syntaxComment",
            "syntaxKeyword",
            "syntaxFunction",
            "syntaxVariable",
            "syntaxString",
            "syntaxNumber",
            "syntaxType",
            "syntaxOperator",
            "syntaxPunctuation",
        ],
        everywhere.clone(),
        Level::Readable,
    );
    each(&["searchMatchText"], vec!["searchMatchBg"], Level::Readable);
    each(
        &["bashMode", "border", "borderAccent"],
        vec!["background"],
        Level::Readable,
    );
    each(&["borderMuted"], vec!["background"], Level::Subtle);
    each(&["mdQuoteBorder", "mdHr"], everywhere, Level::Readable);
    each(&["scrollbarTrack"], vec!["background"], Level::Track);
    each(&["scrollbarThumb"], vec!["scrollbarTrack"], Level::Thumb);
    for (token, level) in THINKING {
        each(&[token], vec!["background"], level);
    }
    rules
}

fn solve_order(rules: &[Rule]) -> Vec<&'static str> {
    fn visit(token: &'static str, rules: &[Rule], order: &mut Vec<&'static str>) {
        if order.contains(&token) {
            return;
        }
        for rule in rules.iter().filter(|rule| rule.token == token) {
            for surface in &rule.on {
                if *surface != "background" {
                    visit(surface, rules, order);
                }
            }
        }
        order.push(token);
    }
    let mut order = Vec::new();
    for rule in rules {
        visit(rule.token, rules, &mut order);
    }
    order
}

pub struct Input {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub palette: Option<[Rgb; 16]>,
    pub saturation: f64,
    pub appearance_hint: Option<Appearance>,
}

pub struct Generated {
    pub colors: Vec<(&'static str, Option<Color>)>,
    pub dim: Vec<&'static str>,
    pub appearance: Option<Appearance>,
}

fn oklab_lightness(color: Rgb) -> f64 {
    rgb_to_oklch(color).l
}

fn relative_luminance(Rgb { r, g, b }: Rgb) -> f64 {
    let linear = |channel: u8| {
        let value = channel as f64 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

fn wcag_contrast(first: Rgb, second: Rgb) -> f64 {
    let a = relative_luminance(first);
    let b = relative_luminance(second);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

const WHITE: Rgb = Rgb::new(255, 255, 255);
const BLACK: Rgb = Rgb::new(0, 0, 0);

pub fn terminal_appearance(background: Rgb, foreground: Option<Rgb>) -> Appearance {
    let white_contrast = wcag_contrast(WHITE, background);
    let black_contrast = wcag_contrast(BLACK, background);
    if let Some(foreground) = foreground {
        let foreground_l = oklab_lightness(foreground);
        let background_l = oklab_lightness(background);
        if (foreground_l - background_l).abs() > 0.05 {
            let appearance = if foreground_l > background_l {
                Appearance::Dark
            } else {
                Appearance::Light
            };
            let best = if appearance == Appearance::Dark {
                white_contrast
            } else {
                black_contrast
            };
            if best >= TEXT_MINIMUM_WCAG_CONTRAST {
                return appearance;
            }
        }
    }
    if white_contrast >= black_contrast {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

fn bell_weight(lightness: f64) -> f64 {
    let gaussian = |x: f64| (-(x - 0.5).powi(2) / (2.0 * 0.25 * 0.25)).exp();
    (gaussian(lightness) - gaussian(0.0)) / (1.0 - gaussian(0.0))
}

fn saturation_curve(family: Family, lightness: f64) -> f64 {
    let floor = if family.max > 0.0 {
        family.min / family.max
    } else {
        1.0
    };
    floor + (1.0 - floor) * bell_weight(lightness)
}

fn level_target(level: Level, appearance: Appearance, surface_l: f64) -> Option<f64> {
    let curve = level_curve(level, appearance);
    if surface_l < curve.reachable.0 || surface_l > curve.reachable.1 {
        return None;
    }
    Some(
        curve
            .coefficients
            .iter()
            .enumerate()
            .fold(0.0, |sum, (power, coefficient)| {
                sum + coefficient * surface_l.powf(power as f64)
            }),
    )
}

fn readable_floor(appearance: Appearance) -> Level {
    match appearance {
        Appearance::Dark => Level::Readable,
        Appearance::Light => Level::Subtle,
    }
}

#[derive(Debug, Clone, Copy)]
struct Source {
    okhsl: Okhsl,
    chroma: f64,
}

fn source_of(color: Rgb) -> Source {
    Source {
        okhsl: rgb_to_okhsl(color),
        chroma: rgb_to_oklch(color).c,
    }
}

fn anchored(source: Source, family: Family, lightness: f64, saturation: f64) -> Rgb {
    let anchor = saturation_curve(family, source.okhsl.l);
    let falloff = if anchor > 0.0 {
        (saturation_curve(family, lightness) / anchor).min(1.0)
    } else {
        1.0
    };
    let color = okhsl_to_rgb(
        source.okhsl.h,
        source.okhsl.s * falloff * saturation,
        lightness,
    );
    let cap = source.chroma * falloff * saturation;
    let oklch = rgb_to_oklch(color);
    if oklch.c <= cap {
        color
    } else {
        oklch_to_rgb(oklch.l, cap, source.okhsl.h)
    }
}

fn with_text_contrast(color: Rgb, surfaces: &[Rgb], lighter: bool) -> Rgb {
    let meets = |candidate: Rgb| {
        surfaces
            .iter()
            .all(|surface| wcag_contrast(candidate, *surface) >= TEXT_MINIMUM_WCAG_CONTRAST)
    };
    if meets(color) {
        return color;
    }
    let Okhsl { h, s, l } = rgb_to_okhsl(color);
    let at = |lightness: f64| okhsl_to_rgb(h, s, lightness);
    let extreme = if lighter { 1.0 } else { 0.0 };
    if !meets(at(extreme)) {
        return at(extreme);
    }
    let (mut low, mut high) = (l, extreme);
    for _ in 0..20 {
        let middle = (low + high) / 2.0;
        if meets(at(middle)) {
            high = middle;
        } else {
            low = middle;
        }
    }
    at(high)
}

fn extreme_of(values: impl Iterator<Item = f64>, lighter: bool) -> f64 {
    values.fold(
        if lighter {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        },
        |best, value| {
            if lighter {
                best.max(value)
            } else {
                best.min(value)
            }
        },
    )
}

pub fn generate(input: &Input) -> Generated {
    let saturation = input.saturation.clamp(0.0, 1.0);
    let Some(background) = input.background else {
        return indexed(saturation, input.appearance_hint);
    };
    let palette = input.palette.map(|palette| palette.map(source_of));
    let appearance = terminal_appearance(background, input.foreground);
    let lighter = appearance == Appearance::Dark;
    let extreme = if lighter { 1.0 } else { 0.0 };
    let background_l = oklab_lightness(background);
    let rules = rules();
    let order = solve_order(&rules);

    let paint = |token: &str, oklab_l: f64| -> Rgb {
        let lightness = oklab_to_okhsl_lightness(oklab_l);
        let family = family_of(token);
        match &palette {
            None => okhsl_to_rgb(
                family.hue,
                (family.min + (family.max - family.min) * bell_weight(lightness)) * saturation,
                lightness,
            ),
            Some(palette) => anchored(palette[slot_of(token)], family, lightness, saturation),
        }
    };

    let target = |level: Level, surface_l: f64, t: f64| -> Option<f64> {
        let reached = level_target(level, appearance, surface_l);
        if reached.is_none() && t == 0.0 {
            return None;
        }
        let distance = reached.unwrap_or(extreme) - surface_l;
        let floor = level_target(readable_floor(appearance), appearance, surface_l)
            .unwrap_or(extreme)
            - surface_l;
        let compressed = if distance.abs() > floor.abs() {
            distance - (distance - floor) * t.min(1.0)
        } else {
            distance
        };
        Some(surface_l + compressed * (1.0 - (t - 1.0).max(0.0)))
    };

    let extreme_text = if lighter { WHITE } else { BLACK };
    let readable = |color: Rgb| wcag_contrast(extreme_text, color) >= TEXT_MINIMUM_WCAG_CONTRAST;
    let limit_panel = |token: &str, l: f64| -> Rgb {
        let color = paint(token, l);
        if readable(color) {
            return color;
        }
        let (mut low, mut high) = (background_l, l);
        for _ in 0..20 {
            let middle = (low + high) / 2.0;
            if readable(paint(token, middle)) {
                low = middle;
            } else {
                high = middle;
            }
        }
        paint(token, low)
    };

    let solve = |t: f64| -> Option<HashMap<&'static str, Rgb>> {
        let mut colors = HashMap::from([("background", background)]);
        for token in &order {
            let mut targets = Vec::new();
            for rule in rules.iter().filter(|rule| rule.token == *token) {
                for surface in &rule.on {
                    let surface_l = oklab_lightness(*colors.get(surface).unwrap_or(&background));
                    let value = target(rule.level, surface_l, t)?;
                    if !(0.0..=1.0).contains(&value) {
                        return None;
                    }
                    targets.push(value);
                }
            }
            let l = extreme_of(targets.into_iter(), lighter);
            let color = if PANELS.contains(token) {
                limit_panel(token, l)
            } else {
                paint(token, l)
            };
            colors.insert(token, color);
        }
        Some(colors)
    };

    let mut relaxation = 0.0;
    let mut colors = solve(0.0);
    if colors.is_none() {
        let (mut low, mut high) = (0.0, 2.0);
        colors = solve(high);
        for _ in 0..20 {
            let middle = (low + high) / 2.0;
            match solve(middle) {
                Some(attempt) => {
                    high = middle;
                    colors = Some(attempt);
                }
                None => low = middle,
            }
        }
        relaxation = high;
    }
    let solved = colors.unwrap_or_default();
    let surfaces_of = |token: &str| -> Vec<Rgb> {
        rules
            .iter()
            .filter(|rule| rule.token == token)
            .flat_map(|rule| {
                rule.on
                    .iter()
                    .map(|surface| *solved.get(surface).unwrap_or(&background))
            })
            .collect()
    };

    let mut result: Vec<(&'static str, Option<Color>)> = TOKEN_FAMILIES
        .into_iter()
        .map(|(token, _)| (token, solved.get(token).map(|color| Color::Rgb(*color))))
        .collect();
    let mut set = |token: &str, color: Option<Color>| {
        if let Some(entry) = result.iter_mut().find(|(name, _)| *name == token) {
            entry.1 = color;
        }
    };

    for token in FOREGROUND_TOKENS {
        let surfaces = surfaces_of(token);
        let mut text = solved.get(token).copied();
        if let Some(foreground) = input.foreground {
            let targets: Vec<Option<f64>> = surfaces
                .iter()
                .map(|surface| target(Level::Emphasis, oklab_lightness(*surface), relaxation))
                .collect();
            if targets
                .iter()
                .all(|value| value.is_some_and(|value| (0.0..=1.0).contains(&value)))
            {
                let needed = extreme_of(targets.into_iter().flatten(), lighter);
                let foreground_l = oklab_lightness(foreground);
                if (lighter && foreground_l >= needed) || (!lighter && foreground_l <= needed) {
                    set(token, None);
                    continue;
                }
                text = Some(anchored(
                    source_of(foreground),
                    NEUTRAL,
                    oklab_to_okhsl_lightness(needed),
                    saturation,
                ));
            }
        }
        if let Some(text) = text {
            set(
                token,
                Some(Color::Rgb(with_text_contrast(text, &surfaces, lighter))),
            );
        }
    }
    Generated {
        colors: result,
        dim: Vec::new(),
        appearance: Some(appearance),
    }
}

fn indexed(saturation: f64, appearance: Option<Appearance>) -> Generated {
    let mut colors = Vec::new();
    let mut dim = Vec::new();
    for (token, family) in TOKEN_FAMILIES.into_iter() {
        if PANELS.contains(&token) {
            colors.push((token, None));
            continue;
        }
        let neutral = family == NEUTRAL;
        colors.push((
            token,
            (!neutral && saturation > 0.0).then(|| Color::Indexed(slot_of(token) as u8)),
        ));
        if neutral && !FOREGROUND_TOKENS.contains(&token) {
            dim.push(token);
        }
    }
    Generated {
        colors,
        dim,
        appearance,
    }
}
