//! The dark theme (`theme/dark.json`) as escape sequences. Colors are upstream's okhsl values
//! converted to RGB by upstream's own `okhslToRgb`; truecolor when `COLORTERM` says so, else the
//! nearest xterm-256 color as upstream's `rgbToAnsi256` picks it.

use std::rc::Rc;
use std::sync::OnceLock;

use pi_tui::components::editor::EditorTheme;
use pi_tui::components::markdown::{DefaultTextStyle, MarkdownTheme, StyleFn};
use pi_tui::components::select_list::SelectListTheme;

/// `(token, r, g, b)` for every token this mode draws with.
const DARK: &[(&str, u8, u8, u8)] = &[
    ("accent", 167, 152, 215),
    ("border", 95, 168, 204),
    ("borderAccent", 160, 142, 213),
    ("borderMuted", 118, 129, 134),
    ("success", 104, 183, 141),
    ("error", 234, 127, 129),
    ("warning", 205, 154, 34),
    ("muted", 157, 165, 169),
    ("dim", 126, 136, 142),
    ("text", 222, 224, 225),
    ("thinkingText", 150, 160, 164),
    ("userMessageBg", 33, 59, 73),
    ("userMessageText", 222, 224, 225),
    ("toolPendingBg", 52, 56, 58),
    ("toolSuccessBg", 37, 65, 49),
    ("toolErrorBg", 91, 40, 42),
    ("toolTitle", 222, 224, 225),
    ("toolOutput", 157, 165, 169),
    ("mdHeading", 205, 154, 34),
    ("mdLink", 105, 173, 208),
    ("mdLinkUrl", 157, 165, 169),
    ("mdCode", 167, 152, 215),
    ("mdCodeBlock", 104, 183, 141),
    ("mdCodeBlockBorder", 157, 165, 169),
    ("mdQuote", 157, 165, 169),
    ("mdQuoteBorder", 157, 165, 169),
    ("mdHr", 157, 165, 169),
    ("mdListBullet", 167, 152, 215),
    ("toolDiffAdded", 104, 183, 141),
    ("toolDiffRemoved", 234, 127, 129),
    ("toolDiffContext", 157, 165, 169),
];

fn truecolor() -> bool {
    static TRUECOLOR: OnceLock<bool> = OnceLock::new();
    *TRUECOLOR.get_or_init(|| {
        let ct = std::env::var("COLORTERM")
            .unwrap_or_default()
            .to_lowercase();
        ct == "truecolor" || ct == "24bit"
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

fn distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let d = |x: u8, y: u8| (x as f64 - y as f64).powi(2);
    d(a.0, b.0) * 0.299 + d(a.1, b.1) * 0.587 + d(a.2, b.2) * 0.114
}

/// Upstream `rgbToAnsi256`: the 6x6x6 cube, or the gray ramp for near-grays closer to it.
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

fn color_ansi(token: &str, background: bool) -> String {
    let &(_, r, g, b) = DARK
        .iter()
        .find(|(name, ..)| *name == token)
        .unwrap_or_else(|| panic!("Unknown theme color: {token}"));
    let layer = if background { 48 } else { 38 };
    if truecolor() {
        format!("\x1b[{layer};2;{r};{g};{b}m")
    } else {
        format!("\x1b[{layer};5;{}m", ansi256(r, g, b))
    }
}

/// `theme.fg(token, text)`.
pub fn fg(token: &str, text: &str) -> String {
    format!("{}{text}\x1b[39m", color_ansi(token, false))
}

/// `theme.bg(token, text)`.
pub fn bg(token: &str, text: &str) -> String {
    format!("{}{text}\x1b[49m", color_ansi(token, true))
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

/// A style function drawing `token` in the foreground.
pub fn fg_fn(token: &'static str) -> StyleFn {
    Rc::new(move |t: &str| fg(token, t))
}

pub fn bg_fn(token: &'static str) -> StyleFn {
    Rc::new(move |t: &str| bg(token, t))
}

/// `getMarkdownTheme()`, without syntax highlighting: code blocks take `mdCodeBlock`, as
/// upstream draws a block whose language it does not know.
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
        highlight_code: Some(Rc::new(|code: &str, _lang: Option<&str>| {
            code.split('\n').map(|l| fg("mdCodeBlock", l)).collect()
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

/// Thinking blocks: `thinkingText`, italic.
pub fn thinking_style() -> DefaultTextStyle {
    DefaultTextStyle {
        color: Some(fg_fn("thinkingText")),
        italic: true,
        ..DefaultTextStyle::default()
    }
}

/// `getEditorTheme()`.
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
}
