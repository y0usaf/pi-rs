mod system;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pi_tui::colors::{
    Color, ColorMode, Rgb, background_ansi, color_to_rgb, foreground_ansi, okhsl_to_rgb,
    oklch_to_rgb, rgb_to_oklab, terminal_color_mode,
};
use pi_tui::components::editor::EditorTheme;
use pi_tui::components::markdown::{DefaultTextStyle, MarkdownTheme, StyleFn};
use pi_tui::components::select_list::SelectListTheme;
use pi_tui::terminal_colors::TerminalColors;
use pi_tui::tui::TuiCore;

pub const SYSTEM_THEME: &str = "system";

const DARK_JSON: &str = include_str!("../../themes/dark.json");
const LIGHT_JSON: &str = include_str!("../../themes/light.json");

const TERMINAL_QUERY_TIMEOUT_MS: u64 = 100;

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
        return Ok(Some(Color::Rgb(Rgb::new(
            channel(0),
            channel(2),
            channel(4),
        ))));
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
        return Ok(Some(Color::Rgb(oklch_to_rgb(l, c, h))));
    }
    if let Some(rest) = lower.strip_prefix("okhsl(") {
        let values = function_args(rest).ok_or_else(invalid)?;
        let h = values[0].0;
        let s = values[1].0 / if values[1].1 == Some('%') { 100.0 } else { 1.0 };
        let l = values[2].0 / if values[2].1 == Some('%') { 100.0 } else { 1.0 };
        if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&l) {
            return Err(invalid());
        }
        return Ok(Some(Color::Rgb(okhsl_to_rgb(h, s, l))));
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
    mode: ColorMode,
    foreground: HashMap<String, String>,
    background: HashMap<String, String>,
    dim: Vec<&'static str>,
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
            total += rgb_to_oklab(color_to_rgb(*color))[0];
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

        let appearance = object
            .get("appearance")
            .and_then(|value| value.as_str())
            .and_then(Appearance::of)
            .or_else(|| detect_appearance(&values))
            .unwrap_or(appearance);
        Ok(Theme::new(
            name,
            source_path,
            appearance,
            &values,
            Vec::new(),
        ))
    }

    fn new(
        name: &str,
        source_path: Option<String>,
        appearance: Appearance,
        values: &HashMap<String, Option<Color>>,
        dim: Vec<&'static str>,
    ) -> Theme {
        let mode = terminal_color_mode();
        let mut foreground = HashMap::new();
        let mut background = HashMap::new();
        for (name, slot, _) in TOKENS {
            let color = values.get(*name).copied().flatten();
            match slot {
                Slot::Foreground => foreground.insert(
                    (*name).to_owned(),
                    color.map_or("\x1b[39m".to_owned(), |color| foreground_ansi(color, mode)),
                ),
                Slot::Background => background.insert(
                    (*name).to_owned(),
                    color.map_or("\x1b[49m".to_owned(), |color| background_ansi(color, mode)),
                ),
            };
        }
        Theme {
            name: name.to_owned(),
            source_path,
            appearance,
            mode,
            foreground,
            background,
            dim,
        }
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
fn color_fg_bg_appearance() -> Option<Appearance> {
    let value = std::env::var("COLORFGBG").ok()?;
    let bg = value.split(';').next_back()?.trim();
    if !(1..=2).contains(&bg.len()) || !bg.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match bg.parse::<u8>().ok()? {
        0..=6 | 8 => Some(Appearance::Dark),
        9..=15 | 7 => Some(Appearance::Light),
        _ => None,
    }
}

pub fn terminal_appearance() -> Appearance {
    let colors = terminal_colors();
    match colors.background {
        Some(background) => system::terminal_appearance(background, colors.foreground),
        None => color_fg_bg_appearance().unwrap_or(Appearance::Dark),
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

fn system_theme() -> Theme {
    let (colors, pending) = TERMINAL.with_borrow(|terminal| {
        (
            terminal.reported.clone().unwrap_or_default(),
            terminal.pending,
        )
    });
    let generated = system::generate(&system::Input {
        foreground: colors.foreground,
        background: colors.background,
        palette: colors.palette,
        saturation: if pending { 0.0 } else { 1.0 },
        appearance_hint: Some(terminal_appearance()),
    });
    let values = generated
        .colors
        .into_iter()
        .map(|(token, color)| (token.to_owned(), color))
        .collect();
    Theme::new(
        SYSTEM_THEME,
        None,
        generated.appearance.unwrap_or_else(terminal_appearance),
        &values,
        generated.dim,
    )
}

pub fn load(name: &str, sources: &Sources) -> Result<Theme, String> {
    if name == SYSTEM_THEME {
        return Ok(system_theme());
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

#[derive(Default)]
struct TerminalState {
    reported: Option<TerminalColors>,
    pending: bool,
    setting: Option<String>,
    sources: Sources,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Theme>>> = const { RefCell::new(None) };
    static TERMINAL: RefCell<TerminalState> = RefCell::default();
}

fn terminal_colors() -> TerminalColors {
    TERMINAL.with_borrow(|terminal| terminal.reported.clone().unwrap_or_default())
}

fn remember(sources: &Sources, setting: Option<&str>) {
    TERMINAL.with_borrow_mut(|terminal| {
        terminal.sources = sources.clone();
        terminal.setting = setting.map(str::to_owned);
    });
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
    TERMINAL.with_borrow_mut(|terminal| terminal.pending = true);
    remember(sources, setting);
    let name =
        resolve_setting(setting, terminal_appearance()).unwrap_or_else(|| SYSTEM_THEME.to_owned());
    match load(&name, sources) {
        Ok(theme) => {
            put(theme);
            None
        }
        Err(error) => {
            put(system_theme());
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
    set(&name, sources)?;
    remember(sources, Some(setting));
    Ok(())
}

/// setTheme: switch to an available theme, leaving the current one in place when it does not load.
pub fn set(name: &str, sources: &Sources) -> Result<(), String> {
    let theme = load(name, sources)?;
    put(theme);
    Ok(())
}

fn apply_terminal_colors(reported: TerminalColors) -> bool {
    let changed = TERMINAL.with_borrow_mut(|terminal| {
        let previous = terminal.reported.clone().unwrap_or_default();
        let next = TerminalColors {
            foreground: reported.foreground.or(previous.foreground),
            background: reported.background.or(previous.background),
            palette: reported.palette.or(previous.palette),
        };
        if terminal.reported.as_ref() == Some(&next) {
            return false;
        }
        terminal.reported = Some(next);
        terminal.pending = false;
        true
    });
    if changed {
        let (setting, sources) =
            TERMINAL.with_borrow(|terminal| (terminal.setting.clone(), terminal.sources.clone()));
        let name = resolve_setting(setting.as_deref(), terminal_appearance())
            .unwrap_or_else(|| SYSTEM_THEME.to_owned());
        if name == SYSTEM_THEME || name != current().name {
            put(load(&name, &sources).unwrap_or_else(|_| system_theme()));
        }
    }
    changed
}

pub fn query_terminal_colors(core: &mut TuiCore, now: u64) -> &'static str {
    core.terminal_colors.start(now, TERMINAL_QUERY_TIMEOUT_MS)
}

pub fn poll_terminal_colors(core: &mut TuiCore, now: u64) -> Option<u64> {
    while let Some(colors) = core.terminal_colors.take(now) {
        if apply_terminal_colors(colors) {
            core.invalidate();
            core.request_render(false);
        }
    }
    core.terminal_colors.deadline()
}

fn ansi(theme: &Theme, name: &str, background: bool) -> String {
    match theme.ansi(name, background) {
        Some(escape) => escape.to_owned(),
        None => panic!("Unknown theme color: {name}"),
    }
}

pub fn color_mode() -> ColorMode {
    current().mode
}

/// theme.fg(token, text).
pub fn fg(name: &str, text: &str) -> String {
    let theme = current();
    let escape = ansi(&theme, name, false);
    if theme.dim.contains(&name) {
        format!("{escape}\x1b[2m{text}\x1b[22;39m")
    } else {
        format!("{escape}{text}\x1b[39m")
    }
}

/// theme.bg(token, text).
pub fn bg(name: &str, text: &str) -> String {
    format!("{}{text}\x1b[49m", ansi(&current(), name, true))
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
