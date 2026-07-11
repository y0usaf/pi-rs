//! Terminal image protocol detection, encoding, sizing, and metadata parsing.
//!
//! Port of `packages/tui/src/terminal-image.ts` from pi v0.79.0.

use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use base64::Engine as _;

const KITTY_PREFIX: &str = "\x1b_G";
const ITERM2_PREFIX: &str = "\x1b]1337;File=";
const KITTY_CHUNK_SIZE: usize = 4096;

/// An inline-image protocol understood by the terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageProtocol {
    Kitty,
    ITerm2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalCapabilities {
    pub images: Option<ImageProtocol>,
    pub true_color: bool,
    pub hyperlinks: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

impl Default for CellDimensions {
    fn default() -> Self {
        Self {
            width_px: 9,
            height_px: 18,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageCellSize {
    pub columns: u32,
    pub rows: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImageRenderOptions {
    pub max_width_cells: Option<u32>,
    pub max_height_cells: Option<u32>,
    pub preserve_aspect_ratio: Option<bool>,
    pub image_id: Option<u32>,
    pub move_cursor: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageRenderResult {
    pub sequence: String,
    pub rows: u32,
    pub image_id: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KittyOptions {
    pub columns: Option<u32>,
    pub rows: Option<u32>,
    pub image_id: Option<u32>,
    pub move_cursor: Option<bool>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ITerm2Options<'a> {
    pub width: Option<&'a str>,
    pub height: Option<&'a str>,
    pub name: Option<&'a str>,
    pub preserve_aspect_ratio: Option<bool>,
    pub inline: Option<bool>,
}

static CELL_WIDTH_PX: AtomicU32 = AtomicU32::new(9);
static CELL_HEIGHT_PX: AtomicU32 = AtomicU32::new(18);
static CAPABILITIES: Mutex<Option<TerminalCapabilities>> = Mutex::new(None);
static IMAGE_ID_FALLBACK: AtomicU32 = AtomicU32::new(1);

pub fn get_cell_dimensions() -> CellDimensions {
    CellDimensions {
        width_px: CELL_WIDTH_PX.load(Ordering::Relaxed),
        height_px: CELL_HEIGHT_PX.load(Ordering::Relaxed),
    }
}

pub fn set_cell_dimensions(dimensions: CellDimensions) {
    CELL_WIDTH_PX.store(dimensions.width_px, Ordering::Relaxed);
    CELL_HEIGHT_PX.store(dimensions.height_px, Ordering::Relaxed);
}

fn value<'a>(env: &'a HashMap<String, String>, key: &str) -> &'a str {
    env.get(key).map_or("", String::as_str)
}

/// Detect capabilities from an explicit environment map.
///
/// Keeping environment access outside this function makes capability tests safe
/// to run in parallel. The callback is invoked only for tmux terminals.
pub fn detect_capabilities(
    env: &HashMap<String, String>,
    tmux_forwards_hyperlinks: impl FnOnce() -> bool,
) -> TerminalCapabilities {
    let term_program = value(env, "TERM_PROGRAM").to_ascii_lowercase();
    let terminal_emulator = value(env, "TERMINAL_EMULATOR").to_ascii_lowercase();
    let term = value(env, "TERM").to_ascii_lowercase();
    let color_term = value(env, "COLORTERM").to_ascii_lowercase();
    let true_color_hint = matches!(color_term.as_str(), "truecolor" | "24bit");

    if env.contains_key("TMUX") || term.starts_with("tmux") {
        return TerminalCapabilities {
            images: None,
            true_color: true_color_hint,
            hyperlinks: tmux_forwards_hyperlinks(),
        };
    }
    if term.starts_with("screen") {
        return TerminalCapabilities {
            images: None,
            true_color: true_color_hint,
            hyperlinks: false,
        };
    }
    if env.contains_key("KITTY_WINDOW_ID") || term_program == "kitty" {
        return known_image_terminal(ImageProtocol::Kitty);
    }
    if term_program == "ghostty"
        || term.contains("ghostty")
        || env.contains_key("GHOSTTY_RESOURCES_DIR")
    {
        return known_image_terminal(ImageProtocol::Kitty);
    }
    if env.contains_key("WEZTERM_PANE") || term_program == "wezterm" {
        return known_image_terminal(ImageProtocol::Kitty);
    }
    if env.contains_key("ITERM_SESSION_ID") || term_program == "iterm.app" {
        return known_image_terminal(ImageProtocol::ITerm2);
    }
    if env.contains_key("WT_SESSION") || matches!(term_program.as_str(), "vscode" | "alacritty") {
        return TerminalCapabilities {
            images: None,
            true_color: true,
            hyperlinks: true,
        };
    }
    if terminal_emulator == "jetbrains-jediterm" {
        return TerminalCapabilities {
            images: None,
            true_color: true,
            hyperlinks: false,
        };
    }
    TerminalCapabilities {
        images: None,
        true_color: true_color_hint,
        hyperlinks: false,
    }
}

fn known_image_terminal(images: ImageProtocol) -> TerminalCapabilities {
    TerminalCapabilities {
        images: Some(images),
        true_color: true,
        hyperlinks: true,
    }
}

fn probe_tmux_hyperlinks() -> bool {
    Command::new("tmux")
        .args(["display-message", "-p", "#{client_termfeatures}"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|features| {
            features
                .split(',')
                .any(|feature| feature.trim() == "hyperlinks")
        })
}

pub fn detect_process_capabilities() -> TerminalCapabilities {
    let env = std::env::vars().collect();
    detect_capabilities(&env, probe_tmux_hyperlinks)
}

pub fn get_capabilities() -> TerminalCapabilities {
    if let Ok(mut cached) = CAPABILITIES.lock() {
        if let Some(capabilities) = *cached {
            return capabilities;
        }
        let capabilities = detect_process_capabilities();
        *cached = Some(capabilities);
        capabilities
    } else {
        detect_process_capabilities()
    }
}

pub fn reset_capabilities_cache() {
    if let Ok(mut cached) = CAPABILITIES.lock() {
        *cached = None;
    }
}

pub fn set_capabilities(capabilities: TerminalCapabilities) {
    if let Ok(mut cached) = CAPABILITIES.lock() {
        *cached = Some(capabilities);
    }
}

pub fn is_image_line(line: &str) -> bool {
    line.contains(KITTY_PREFIX) || line.contains(ITERM2_PREFIX)
}

/// Allocate a nonzero Kitty image id.
pub fn allocate_image_id() -> u32 {
    let mut bytes = [0_u8; 4];
    if getrandom::fill(&mut bytes).is_ok() {
        return u32::from_ne_bytes(bytes).max(1);
    }
    IMAGE_ID_FALLBACK.fetch_add(1, Ordering::Relaxed).max(1)
}

pub fn encode_kitty(base64_data: &str, options: KittyOptions) -> String {
    let mut params = vec!["a=T".to_owned(), "f=100".to_owned(), "q=2".to_owned()];
    if options.move_cursor == Some(false) {
        params.push("C=1".to_owned());
    }
    if let Some(columns) = options.columns.filter(|value| *value != 0) {
        params.push(format!("c={columns}"));
    }
    if let Some(rows) = options.rows.filter(|value| *value != 0) {
        params.push(format!("r={rows}"));
    }
    if let Some(image_id) = options.image_id.filter(|value| *value != 0) {
        params.push(format!("i={image_id}"));
    }
    let params = params.join(",");
    if base64_data.len() <= KITTY_CHUNK_SIZE {
        return format!("{KITTY_PREFIX}{params};{base64_data}\x1b\\");
    }

    let mut sequence = String::new();
    let chunks = base64_data.as_bytes().chunks(KITTY_CHUNK_SIZE);
    let chunk_count = chunks.len();
    for (index, chunk) in chunks.enumerate() {
        // Base64 is ASCII, but avoid a production unwrap if a caller supplies
        // arbitrary text and a chunk boundary lands within a UTF-8 scalar.
        let data = String::from_utf8_lossy(chunk);
        if index == 0 {
            sequence.push_str(&format!("{KITTY_PREFIX}{params},m=1;{data}\x1b\\"));
        } else if index + 1 == chunk_count {
            sequence.push_str(&format!("{KITTY_PREFIX}m=0;{data}\x1b\\"));
        } else {
            sequence.push_str(&format!("{KITTY_PREFIX}m=1;{data}\x1b\\"));
        }
    }
    sequence
}

pub fn delete_kitty_image(image_id: u32) -> String {
    format!("\x1b_Ga=d,d=I,i={image_id},q=2\x1b\\")
}

pub fn delete_all_kitty_images() -> &'static str {
    "\x1b_Ga=d,d=A,q=2\x1b\\"
}

pub fn encode_iterm2(base64_data: &str, options: ITerm2Options<'_>) -> String {
    let inline = u8::from(options.inline != Some(false));
    let mut params = vec![format!("inline={inline}")];
    if let Some(width) = options.width {
        params.push(format!("width={width}"));
    }
    if let Some(height) = options.height {
        params.push(format!("height={height}"));
    }
    if let Some(name) = options.name.filter(|name| !name.is_empty()) {
        params.push(format!(
            "name={}",
            base64::engine::general_purpose::STANDARD.encode(name)
        ));
    }
    if options.preserve_aspect_ratio == Some(false) {
        params.push("preserveAspectRatio=0".to_owned());
    }
    format!("{ITERM2_PREFIX}{}:{base64_data}\x07", params.join(";"))
}

pub fn calculate_image_cell_size(
    image_dimensions: ImageDimensions,
    max_width_cells: u32,
    max_height_cells: Option<u32>,
    cell_dimensions: CellDimensions,
) -> ImageCellSize {
    let max_width = max_width_cells.max(1);
    let max_height = max_height_cells.map(|height| height.max(1));
    let image_width = image_dimensions.width_px.max(1);
    let image_height = image_dimensions.height_px.max(1);
    let cell_width = cell_dimensions.width_px.max(1);
    let cell_height = cell_dimensions.height_px.max(1);

    let width_scale = f64::from(max_width * cell_width) / f64::from(image_width);
    let height_scale = max_height.map_or(width_scale, |height| {
        f64::from(height * cell_height) / f64::from(image_height)
    });
    let scale = width_scale.min(height_scale);
    let columns = (f64::from(image_width) * scale / f64::from(cell_width)).ceil() as u32;
    let rows = (f64::from(image_height) * scale / f64::from(cell_height)).ceil() as u32;

    ImageCellSize {
        columns: columns.clamp(1, max_width),
        rows: rows.clamp(1, max_height.unwrap_or(rows.max(1))),
    }
}

pub fn calculate_image_rows(
    image_dimensions: ImageDimensions,
    target_width_cells: u32,
    cell_dimensions: CellDimensions,
) -> u32 {
    calculate_image_cell_size(image_dimensions, target_width_cells, None, cell_dimensions).rows
}

fn decode_base64(data: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

pub fn get_png_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let data = decode_base64(base64_data)?;
    if data.len() < 24 || data.get(..4) != Some(&[0x89, b'P', b'N', b'G']) {
        return None;
    }
    Some(ImageDimensions {
        width_px: u32::from_be_bytes(data.get(16..20)?.try_into().ok()?),
        height_px: u32::from_be_bytes(data.get(20..24)?.try_into().ok()?),
    })
}

pub fn get_jpeg_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let data = decode_base64(base64_data)?;
    if data.get(..2) != Some(&[0xff, 0xd8]) {
        return None;
    }
    let mut offset = 2;
    while offset < data.len().saturating_sub(9) {
        if data.get(offset) != Some(&0xff) {
            offset += 1;
            continue;
        }
        let marker = *data.get(offset + 1)?;
        if (0xc0..=0xc2).contains(&marker) {
            return Some(ImageDimensions {
                height_px: u32::from(u16::from_be_bytes(
                    data.get(offset + 5..offset + 7)?.try_into().ok()?,
                )),
                width_px: u32::from(u16::from_be_bytes(
                    data.get(offset + 7..offset + 9)?.try_into().ok()?,
                )),
            });
        }
        let length = usize::from(u16::from_be_bytes(
            data.get(offset + 2..offset + 4)?.try_into().ok()?,
        ));
        if length < 2 {
            return None;
        }
        offset = offset.saturating_add(2 + length);
    }
    None
}

pub fn get_gif_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let data = decode_base64(base64_data)?;
    if data.len() < 10 || !matches!(data.get(..6), Some(b"GIF87a" | b"GIF89a")) {
        return None;
    }
    Some(ImageDimensions {
        width_px: u32::from(u16::from_le_bytes(data.get(6..8)?.try_into().ok()?)),
        height_px: u32::from(u16::from_le_bytes(data.get(8..10)?.try_into().ok()?)),
    })
}

pub fn get_webp_dimensions(base64_data: &str) -> Option<ImageDimensions> {
    let data = decode_base64(base64_data)?;
    if data.len() < 30 || data.get(..4) != Some(b"RIFF") || data.get(8..12) != Some(b"WEBP") {
        return None;
    }
    match data.get(12..16)? {
        b"VP8 " => Some(ImageDimensions {
            width_px: u32::from(u16::from_le_bytes(data.get(26..28)?.try_into().ok()?) & 0x3fff),
            height_px: u32::from(u16::from_le_bytes(data.get(28..30)?.try_into().ok()?) & 0x3fff),
        }),
        b"VP8L" => {
            let bits = u32::from_le_bytes(data.get(21..25)?.try_into().ok()?);
            Some(ImageDimensions {
                width_px: (bits & 0x3fff) + 1,
                height_px: ((bits >> 14) & 0x3fff) + 1,
            })
        }
        b"VP8X" => Some(ImageDimensions {
            width_px: (u32::from(data[24])
                | (u32::from(data[25]) << 8)
                | (u32::from(data[26]) << 16))
                + 1,
            height_px: (u32::from(data[27])
                | (u32::from(data[28]) << 8)
                | (u32::from(data[29]) << 16))
                + 1,
        }),
        _ => None,
    }
}

pub fn get_image_dimensions(base64_data: &str, mime_type: &str) -> Option<ImageDimensions> {
    match mime_type {
        "image/png" => get_png_dimensions(base64_data),
        "image/jpeg" => get_jpeg_dimensions(base64_data),
        "image/gif" => get_gif_dimensions(base64_data),
        "image/webp" => get_webp_dimensions(base64_data),
        _ => None,
    }
}

pub fn render_image_with_protocol(
    protocol: ImageProtocol,
    base64_data: &str,
    image_dimensions: ImageDimensions,
    options: ImageRenderOptions,
) -> ImageRenderResult {
    let size = calculate_image_cell_size(
        image_dimensions,
        options.max_width_cells.unwrap_or(80),
        options.max_height_cells,
        get_cell_dimensions(),
    );
    match protocol {
        ImageProtocol::Kitty => ImageRenderResult {
            sequence: encode_kitty(
                base64_data,
                KittyOptions {
                    columns: Some(size.columns),
                    rows: Some(size.rows),
                    image_id: options.image_id,
                    move_cursor: options.move_cursor,
                },
            ),
            rows: size.rows,
            image_id: options.image_id,
        },
        ImageProtocol::ITerm2 => ImageRenderResult {
            sequence: encode_iterm2(
                base64_data,
                ITerm2Options {
                    width: Some(&size.columns.to_string()),
                    height: Some("auto"),
                    preserve_aspect_ratio: Some(options.preserve_aspect_ratio.unwrap_or(true)),
                    ..ITerm2Options::default()
                },
            ),
            rows: size.rows,
            image_id: None,
        },
    }
}

pub fn render_image(
    base64_data: &str,
    image_dimensions: ImageDimensions,
    options: ImageRenderOptions,
) -> Option<ImageRenderResult> {
    let protocol = get_capabilities().images?;
    Some(render_image_with_protocol(
        protocol,
        base64_data,
        image_dimensions,
        options,
    ))
}

pub fn hyperlink(text: &str, url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

pub fn image_fallback(
    mime_type: &str,
    dimensions: Option<ImageDimensions>,
    filename: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if let Some(filename) = filename.filter(|name| !name.is_empty()) {
        parts.push(filename.to_owned());
    }
    parts.push(format!("[{mime_type}]"));
    if let Some(dimensions) = dimensions {
        parts.push(format!("{}x{}", dimensions.width_px, dimensions.height_px));
    }
    format!("[Image: {}]", parts.join(" "))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn env(values: &[(&str, &str)]) -> HashMap<String, String> {
        values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn capability_detection_matches_known_terminals_and_mux_precedence() {
        assert_eq!(
            detect_capabilities(&env(&[]), || true),
            TerminalCapabilities {
                images: None,
                true_color: false,
                hyperlinks: false
            }
        );
        assert_eq!(
            detect_capabilities(&env(&[("TERM_PROGRAM", "Ghostty")]), || false),
            known_image_terminal(ImageProtocol::Kitty)
        );
        assert_eq!(
            detect_capabilities(&env(&[("TERM_PROGRAM", "iTerm.app")]), || false),
            known_image_terminal(ImageProtocol::ITerm2)
        );
        assert_eq!(
            detect_capabilities(
                &env(&[
                    ("TMUX", "1"),
                    ("TERM_PROGRAM", "ghostty"),
                    ("COLORTERM", "truecolor")
                ]),
                || true
            ),
            TerminalCapabilities {
                images: None,
                true_color: true,
                hyperlinks: true
            }
        );
        assert!(!detect_capabilities(&env(&[("TERM", "screen-256color")]), || true).hyperlinks);
        assert_eq!(
            detect_capabilities(&env(&[("TERMINAL_EMULATOR", "JetBrains-JediTerm")]), || {
                true
            }),
            TerminalCapabilities {
                images: None,
                true_color: true,
                hyperlinks: false
            }
        );
    }

    #[test]
    fn image_lines_are_detected_anywhere_without_false_positives() {
        assert!(is_image_line("text \x1b]1337;File=inline=1:data\x07 tail"));
        assert!(is_image_line(&format!(
            "prefix \x1b_G{} suffix",
            "A".repeat(300_000)
        )));
        assert!(!is_image_line("plain ]1337;File and _G text"));
        assert!(!is_image_line("\x1b[31mred\x1b[0m"));
    }

    #[test]
    fn sizing_honors_pixel_aspect_and_height_limit() {
        assert_eq!(
            calculate_image_cell_size(
                ImageDimensions {
                    width_px: 10,
                    height_px: 100
                },
                10,
                Some(5),
                CellDimensions {
                    width_px: 10,
                    height_px: 10
                }
            ),
            ImageCellSize {
                columns: 1,
                rows: 5
            }
        );
        assert_eq!(
            calculate_image_rows(
                ImageDimensions {
                    width_px: 100,
                    height_px: 100
                },
                10,
                CellDimensions::default()
            ),
            5
        );
    }

    #[test]
    fn kitty_encoding_chunks_and_deletes() {
        assert_eq!(
            encode_kitty(
                "AAAA",
                KittyOptions {
                    columns: Some(2),
                    rows: Some(2),
                    move_cursor: Some(false),
                    ..KittyOptions::default()
                }
            ),
            "\x1b_Ga=T,f=100,q=2,C=1,c=2,r=2;AAAA\x1b\\"
        );
        let encoded = encode_kitty(&"A".repeat(8193), KittyOptions::default());
        assert!(encoded.starts_with("\x1b_Ga=T,f=100,q=2,m=1;"));
        assert!(encoded.contains("\x1b_Gm=1;"));
        assert!(encoded.ends_with("\x1b_Gm=0;A\x1b\\"));
        assert_eq!(delete_kitty_image(42), "\x1b_Ga=d,d=I,i=42,q=2\x1b\\");
        assert_eq!(delete_all_kitty_images(), "\x1b_Ga=d,d=A,q=2\x1b\\");
    }

    #[test]
    fn iterm_hyperlink_and_fallback_encoding_match_spec() {
        assert_eq!(
            encode_iterm2(
                "AAAA",
                ITerm2Options {
                    width: Some("2"),
                    height: Some("auto"),
                    name: Some("x.png"),
                    preserve_aspect_ratio: Some(false),
                    ..ITerm2Options::default()
                }
            ),
            "\x1b]1337;File=inline=1;width=2;height=auto;name=eC5wbmc=;preserveAspectRatio=0:AAAA\x07"
        );
        assert_eq!(
            hyperlink("click", "https://example.com"),
            "\x1b]8;;https://example.com\x1b\\click\x1b]8;;\x1b\\"
        );
        assert_eq!(
            image_fallback(
                "image/png",
                Some(ImageDimensions {
                    width_px: 2,
                    height_px: 3
                }),
                Some("x.png")
            ),
            "[Image: x.png [image/png] 2x3]"
        );
    }

    #[test]
    fn parses_png_jpeg_gif_and_webp_dimensions() {
        fn b64(bytes: &[u8]) -> String {
            base64::engine::general_purpose::STANDARD.encode(bytes)
        }
        let mut png = vec![0; 24];
        png[..4].copy_from_slice(&[0x89, b'P', b'N', b'G']);
        png[16..20].copy_from_slice(&640_u32.to_be_bytes());
        png[20..24].copy_from_slice(&480_u32.to_be_bytes());
        assert_eq!(
            get_png_dimensions(&b64(&png)),
            Some(ImageDimensions {
                width_px: 640,
                height_px: 480
            })
        );
        let jpeg = [0xff, 0xd8, 0xff, 0xc0, 0, 7, 8, 0x01, 0xe0, 0x02, 0x80, 0];
        assert_eq!(
            get_jpeg_dimensions(&b64(&jpeg)),
            Some(ImageDimensions {
                width_px: 640,
                height_px: 480
            })
        );
        let mut gif = *b"GIF89a\0\0\0\0";
        gif[6..8].copy_from_slice(&320_u16.to_le_bytes());
        gif[8..10].copy_from_slice(&200_u16.to_le_bytes());
        assert_eq!(
            get_gif_dimensions(&b64(&gif)),
            Some(ImageDimensions {
                width_px: 320,
                height_px: 200
            })
        );
        let mut webp = vec![0; 30];
        webp[..4].copy_from_slice(b"RIFF");
        webp[8..12].copy_from_slice(b"WEBP");
        webp[12..16].copy_from_slice(b"VP8X");
        webp[24..27].copy_from_slice(&[0x7f, 0x02, 0]); // stored width is 639
        webp[27..30].copy_from_slice(&[0xdf, 0x01, 0]); // stored height is 479
        assert_eq!(
            get_webp_dimensions(&b64(&webp)),
            Some(ImageDimensions {
                width_px: 640,
                height_px: 480
            })
        );
        assert_eq!(get_image_dimensions("bad", "image/png"), None);
        assert_eq!(get_image_dimensions(&b64(&png), "image/bmp"), None);
    }
}
