//! Port of `cli/file-processor.ts` — process `@file` CLI arguments into
//! text content and image attachments, plus `cli/initial-message.ts`
//! (`buildInitialMessage`) composition.
//!
//! The text path is pinned byte-for-byte against Pi's real
//! `processFileArguments`/`buildInitialMessage` by the differential oracle
//! `tests/file-processor-parity/oracle.json` (`crates/pi-rs-app/tests/
//! file_processor_parity.rs`). The image path (auto-resize, dimension note)
//! mirrors `utils/image-resize.ts` through the same `pi_rs_host::image`
//! mechanism the read tool uses.

use std::io::IsTerminal;
use std::path::Path;

use pi_rs_host::image::{self, ImageResizeOptions, ResizedImage};

/// A single processed `@file` result: accumulated text and image attachments
/// (spec `ProcessedFiles`).
#[derive(Debug)]
pub struct ProcessedFiles {
    pub text: String,
    pub images: Vec<ImageContent>,
}

/// Spec `ImageContent` (ai `ImageContent`): image attachment.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ImageContent {
    pub mime_type: String,
    pub data: String, // base64
}

/// Spec `buildInitialMessage` result.
pub struct InitialMessage {
    pub initial_message: Option<String>,
    pub initial_images: Option<Vec<ImageContent>>,
}

/// Spec `detectSupportedImageMimeType` — the checks used must match
/// `utils/mime.ts`. We reuse the same sniff logic the read tool's Lua side
/// uses by delegating to `pi_rs_host`'s image/mime helpers where available;
/// here we reimplement the byte sniff directly (the Lua `mime.lua` port keeps
/// the identical algorithm, so a text-file oracle plus a sniff-driven image
/// decision stays faithful).
///
/// Pi's `detectSupportedImageMimeTypeFromFile` only ever sniffs the first
/// `IMAGE_TYPE_SNIFF_BYTES` bytes of the file (`mime.ts`), so callers must
/// hand us a window truncated to that length — otherwise an animated chunk
/// (`acTL`) beyond the sniff window would be scored differently than Pi.
pub const IMAGE_TYPE_SNIFF_BYTES: usize = 4100;

fn detect_supported_image_mime_type(buf: &[u8]) -> Option<&'static str> {
    const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
    // Pi: `startsWith(buffer, [0xff, 0xd8, 0xff])` (no length guard beyond the
    // 3 magic bytes) then `buffer[3] === 0xf7 ? null : "image/jpeg"`.
    if buf.len() >= 3 && buf[0] == 0xFF && buf[1] == 0xD8 && buf[2] == 0xFF {
        // F7 (JPG) is not a supported baseline and is rejected. For a 3-byte
        // "FF D8 FF" buffer Pi's `buffer[3]` reads `undefined !== 0xf7`, so it
        // yields `image/jpeg`; `buf.get(3)` below reproduces that.
        if buf.get(3) == Some(&0xF7) {
            return None;
        }
        return Some("image/jpeg");
    }
    if buf.starts_with(&PNG_SIG) {
        if is_static_png(buf) {
            return Some("image/png");
        }
        return None;
    }
    if buf.len() >= 3 && &buf[0..3] == b"GIF" {
        return Some("image/gif");
    }
    if buf.len() >= 12 && &buf[0..4] == b"RIFF" && &buf[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// `readUint32BE` mirror of `mime.ts` — out-of-range reads yield 0 (JS
/// `(buf[offset] ?? 0)`), so the snapshot of Pi's checks behaves identically
/// on short buffers.
fn read_uint32_be(buf: &[u8], offset: usize) -> u32 {
    let b = |i: usize| buf.get(i).copied().unwrap_or(0) as u32;
    (b(offset) << 24) | (b(offset + 1) << 16) | (b(offset + 2) << 8) | b(offset + 3)
}

/// Reject animated PNGs (unreliable inline rendering) — spec `mime.ts`.
/// This is the conjunction `isPng(buffer) && !isAnimatedPng(buffer)`; the
/// buffer here is already the caller's sniff window, truncated to
/// `IMAGE_TYPE_SNIFF_BYTES`, matching Pi's
/// `detectSupportedImageMimeTypeFromFile`.
///
/// The fall-through semantics matter for parity: Pi's `isAnimatedPng` returns
/// `false` (i.e. static → png) both when it hits `IDAT` *and* when the chunk
/// walk runs off the end of the sniff window or ends without finding `acTL`.
/// Only an `acTL` chunk makes it animated. So a PNG whose `IDAT` lies beyond
/// the 4100-byte window is still `image/png` in Pi.
fn is_static_png(buf: &[u8]) -> bool {
    // Pi `isPng`: length >= 16, IHDR data length field (u32 at offset 8, right
    // after the 8-byte signature) === 13, and chunk type at offset 12..16 is
    // `IHDR`.
    if buf.len() < 16 {
        return false;
    }
    if read_uint32_be(buf, 8) != 13 {
        return false;
    }
    if &buf[12..16] != b"IHDR" {
        return false;
    }
    // Pi `isAnimatedPng`: walk chunks after the 8-byte signature.
    let mut offset = 8usize;
    while offset + 8 <= buf.len() {
        let chunk_len = u32::from_be_bytes([
            buf[offset],
            buf[offset + 1],
            buf[offset + 2],
            buf[offset + 3],
        ]) as usize;
        let chunk_type = &buf[offset + 4..offset + 8];
        if chunk_type == b"acTL" {
            return false; // animated → not a static png
        }
        if chunk_type == b"IDAT" {
            return true; // static
        }
        let next = offset + 8 + chunk_len + 4;
        if next <= offset || next > buf.len() {
            // Pi returns false from isAnimatedPng here → static → png.
            return true;
        }
        offset = next;
    }
    // Loop ended without finding acTL → Pi's isAnimatedPng is false → static.
    true
}

/// Expand `~` and resolve relative to `base_dir` exactly as the spec's
/// `resolveReadPath` (via `resolveToCwd` + `normalizePath` with
/// `stripAtPrefix`/`normalizeUnicodeSpaces`). Used by file processing only
/// for `@file` paths (the strip-`@` is handled by the caller applying the
/// spec's `args.ts` parsing).
fn resolve_read_path(file_path: &str, base_dir: &Path) -> std::path::PathBuf {
    let expanded = if file_path == "~" {
        std::env::var("HOME").unwrap_or_default()
    } else if let Some(rest) = file_path.strip_prefix("~/") {
        std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join(rest)
            .to_string_lossy()
            .into_owned()
    } else {
        file_path.to_owned()
    };
    let p = Path::new(&expanded);
    if p.is_absolute() {
        // Normalize . / .. and duplicate separators.
        let mut out = std::path::PathBuf::new();
        for comp in p.components() {
            match comp {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other.as_os_str()),
            }
        }
        let str = out.to_string_lossy().into_owned();
        if str.starts_with("//") {
            // node resolve produces "//host/path" for leading double slash.
            return std::path::PathBuf::from(str);
        }
        return out;
    }
    let joined = base_dir.join(expanded);
    let mut out = std::path::PathBuf::new();
    for comp in joined.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `utils/image-resize.ts formatDimensionNote`: coordinate-mapping note for
/// resized images (JS `${n}` interpolation; toFixed(2) for scale).
fn format_dimension_note(resized: &ResizedImage) -> Option<String> {
    if !resized.was_resized {
        return None;
    }
    let scale = resized.original_width as f64 / resized.width as f64;
    Some(format!(
        "[Image: original {:.0}x{:.0}, displayed at {:.0}x{:.0}. Multiply coordinates by {:.2} to map to original image.]",
        resized.original_width, resized.original_height, resized.width, resized.height, scale
    ))
}

/// Spec `processFileArguments(fileArgs, { autoResizeImages })`, the text and
/// image path. Returns the accumulated text + image attachments. On a missing
/// file, Pi prints `Error: File not found: <abs>` to stderr and exits 1 — the
/// caller (main.rs) owns that process-exit contract; here we signal it by
/// returning a `Err` carrying the absolute path.
pub async fn process_file_arguments(
    file_args: &[String],
    base_dir: &Path,
    auto_resize_images: bool,
) -> Result<ProcessedFiles, FileProcessorError> {
    let mut text = String::new();
    let mut images: Vec<ImageContent> = Vec::new();

    for file_arg in file_args {
        let absolute_path = resolve_read_path(file_arg, base_dir);

        if !absolute_path.exists() {
            return Err(FileProcessorError::NotFound(absolute_path));
        }
        let stats = match std::fs::metadata(&absolute_path) {
            Ok(m) => m,
            Err(e) => return Err(FileProcessorError::Other(format!("{e}"))),
        };
        if stats.len() == 0 {
            // Skip empty files.
            continue;
        }

        let bytes = match std::fs::read(&absolute_path) {
            Ok(b) => b,
            Err(e) => {
                return Err(FileProcessorError::ReadFailed {
                    path: absolute_path.clone(),
                    message: e.to_string(),
                });
            }
        };
        // Pi sniffs only the first `IMAGE_TYPE_SNIFF_BYTES` bytes
        // (`mime.ts::detectSupportedImageMimeTypeFromFile`); the full file is
        // read later for the image payload / text content.
        let sniff = &bytes[..bytes.len().min(IMAGE_TYPE_SNIFF_BYTES)];
        let mime_type = detect_supported_image_mime_type(sniff);

        if let Some(mime) = mime_type {
            let content = bytes;
            let attachment: ImageContent;
            let dimension_note: Option<String>;

            if auto_resize_images {
                // Lightweight resize runs on the blocking pool (spec: Photon
                // worker); Rust thread-blocking approximation is fine.
                let resized = tokio::task::spawn_blocking(move || {
                    image::resize_image(
                        &content,
                        mime,
                        ImageResizeOptions {
                            max_width: None,
                            max_height: None,
                            max_bytes: None,
                            jpeg_quality: None,
                        },
                    )
                })
                .await
                .ok()
                .flatten();
                let Some(resized) = resized else {
                    text.push_str(&format!(
                        "<file name=\"{}\">[Image omitted: could not be resized below the inline image size limit.]</file>\n",
                        absolute_path.display()
                    ));
                    continue;
                };
                dimension_note = format_dimension_note(&resized);
                attachment = ImageContent {
                    mime_type: resized.mime_type.clone(),
                    data: resized.data.clone(),
                };
            } else {
                dimension_note = None;
                use base64::Engine as _;
                use base64::engine::general_purpose::STANDARD as B64;
                attachment = ImageContent {
                    mime_type: mime.to_owned(),
                    data: B64.encode(&content),
                };
            }

            images.push(attachment);
            if let Some(note) = dimension_note {
                text.push_str(&format!(
                    "<file name=\"{}\">{}</file>\n",
                    absolute_path.display(),
                    note
                ));
            } else {
                text.push_str(&format!(
                    "<file name=\"{}\"></file>\n",
                    absolute_path.display()
                ));
            }
        } else {
            // Text file.
            match std::str::from_utf8(&bytes) {
                Ok(content) => {
                    text.push_str(&format!(
                        "<file name=\"{}\">\n{}\n</file>\n",
                        absolute_path.display(),
                        content
                    ));
                }
                Err(_) => {
                    let content = String::from_utf8_lossy(&bytes);
                    text.push_str(&format!(
                        "<file name=\"{}\">\n{}\n</file>\n",
                        absolute_path.display(),
                        content
                    ));
                }
            }
        }
    }

    Ok(ProcessedFiles { text, images })
}

/// Error variants for `@file` processing. The missing-file / read-failed
/// cases map in main.rs to Pi's `console.error` + `process.exit(1)`.
#[derive(Debug)]
pub enum FileProcessorError {
    NotFound(std::path::PathBuf),
    ReadFailed {
        path: std::path::PathBuf,
        message: String,
    },
    Other(String),
}

/// Spec `cli/initial-message.ts buildInitialMessage` — combine stdin content,
/// `@file` text, and the first CLI message into one initial prompt.
pub fn build_initial_message(
    messages: &mut Vec<String>,
    file_text: &str,
    file_images: Vec<ImageContent>,
    stdin_content: Option<String>,
) -> InitialMessage {
    let mut parts: Vec<String> = Vec::new();
    if let Some(stdin) = stdin_content {
        parts.push(stdin);
    }
    if !file_text.is_empty() {
        parts.push(file_text.to_owned());
    }
    if !messages.is_empty() {
        parts.push(messages.remove(0));
    }
    InitialMessage {
        initial_message: if parts.is_empty() {
            None
        } else {
            Some(parts.join(""))
        },
        initial_images: if file_images.is_empty() {
            None
        } else {
            Some(file_images)
        },
    }
}

/// Spec `main.ts readPipedStdin` — returns `None` when stdin is a TTY;
/// otherwise reads all piped stdin and trims it. `None` if empty after trim.
pub fn read_piped_stdin() -> Option<String> {
    if std::io::stdin().is_terminal() {
        return None;
    }
    let mut data = String::new();
    use std::io::Read;
    let mut stdin = std::io::stdin();
    if stdin.read_to_string(&mut data).is_err() {
        return None;
    }
    let trimmed = data.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_read_path_abs_and_rel() {
        let mkdir = std::env::temp_dir().join(format!("pi-fp-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&mkdir);
        let p = resolve_read_path("foo/../bar.txt", &mkdir);
        assert!(p.ends_with("bar.txt"));
    }

    #[test]
    fn build_init_message_order() {
        let mut msgs = vec!["hi".to_owned()];
        let out = build_initial_message(&mut msgs, "FILE\n", vec![], Some("STDIN\n".to_owned()));
        assert_eq!(out.initial_message.as_deref(), Some("STDIN\nFILE\nhi"));
        // first message consumed
        assert!(msgs.is_empty());
        let mut msgs2: Vec<String> = vec![];
        let out2 = build_initial_message(&mut msgs2, "", vec![], None);
        assert!(out2.initial_message.is_none());
    }

    /// A minimal valid static PNG: signature + IHDR (data length 13) + IDAT.
    /// Each chunk is length(4) + name(4) + data + CRC(4), matching the layout
    /// Pi's `isAnimatedPng` walk expects.
    fn static_png() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]); // signature
        b.extend_from_slice(&13u32.to_be_bytes()); // IHDR length
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&[0u8; 13]); // IHDR payload
        b.extend_from_slice(&[0u8; 4]); // IHDR CRC
        b.extend_from_slice(&4u32.to_be_bytes()); // IDAT length
        b.extend_from_slice(b"IDAT");
        b.extend_from_slice(&[0u8; 4]); // IDAT payload
        b.extend_from_slice(&[0u8; 4]); // IDAT CRC
        b
    }

    #[test]
    fn sniff_static_png_is_image() {
        let b = static_png();
        assert_eq!(detect_supported_image_mime_type(&b), Some("image/png"));
    }

    #[test]
    fn sniff_missing_ihdr_length_is_not_png() {
        let mut b = static_png();
        // Corrupt IHDR data-length (Pi requires exactly 13) -> not a PNG.
        b[8] = 12;
        assert_eq!(detect_supported_image_mime_type(&b), None);
    }

    // IDAT length field begins after sig(8) + IHDR len(4) + "IHDR"(4) + payload(13)
    // + IHDR CRC(4).
    const IDAT_LEN_OFFSET: usize = 8 + 4 + 4 + 13 + 4;

    /// Build a PNG whose acTL chunk sits beyond the 4100-byte sniff window,
    /// hidden behind a large auxiliary chunk. Walking the whole file would see
    /// the acTL (animated); Pi — and the port — truncate to the first
    /// `IMAGE_TYPE_SNIFF_BYTES` bytes, so it is treated as static.
    fn static_png_aclt_beyond_window() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]); // signature
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&[0u8; 13]);
        b.extend_from_slice(&[0u8; 4]); // IHDR CRC
        // Big auxiliary chunk (tEXt) so the walk truncates inside it.
        b.extend_from_slice(&5000u32.to_be_bytes());
        b.extend_from_slice(b"tEXt");
        b.extend_from_slice(&[0u8; 5000]);
        b.extend_from_slice(&[0u8; 4]); // tEXt CRC
        // acTL beyond the window.
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(b"acTL");
        b.extend_from_slice(&[0u8; 4]); // acTL CRC
        b.extend_from_slice(&4u32.to_be_bytes());
        b.extend_from_slice(b"IDAT");
        b.extend_from_slice(&[0u8; 4]);
        b
    }

    #[test]
    fn sniff_apng_beyond_sniff_window_is_static() {
        // The whole file has an acTL → animated, but it is past the sniff
        // window so Pi reads only the first 4100 bytes and treats it as png.
        // The truncation is the caller's job (process_file_arguments passes
        // `&bytes[..min(len, IMAGE_TYPE_SNIFF_BYTES)]`), so we simulate it.
        let b = static_png_aclt_beyond_window();
        let sniff = &b[..b.len().min(IMAGE_TYPE_SNIFF_BYTES)];
        assert_eq!(detect_supported_image_mime_type(sniff), Some("image/png"));
    }

    #[test]
    fn sniff_apng_within_sniff_window_is_not_png() {
        // acTL inside the sniff window -> animated -> rejected.
        let mut b = static_png();
        // Splice a zero-length acTL chunk between IHDR and IDAT.
        b.splice(IDAT_LEN_OFFSET..IDAT_LEN_OFFSET, [0u8, 0, 0, 0]);
        b.splice(
            IDAT_LEN_OFFSET + 4..IDAT_LEN_OFFSET + 4,
            b"acTL".iter().copied(),
        );
        assert_eq!(detect_supported_image_mime_type(&b), None);
    }

    #[test]
    fn sniff_jpeg_and_three_byte_jpeg() {
        // FF D8 FF E0 -> jpeg
        assert_eq!(
            detect_supported_image_mime_type(&[0xff, 0xd8, 0xff, 0xe0]),
            Some("image/jpeg")
        );
        // FF D8 FF F7 -> JPG, rejected
        assert_eq!(
            detect_supported_image_mime_type(&[0xff, 0xd8, 0xff, 0xf7]),
            None
        );
        // Pi allows a 3-byte "FF D8 FF" as jpeg (buffer[3] is undefined, not
        // 0xf7).
        assert_eq!(
            detect_supported_image_mime_type(&[0xff, 0xd8, 0xff]),
            Some("image/jpeg")
        );
    }

    #[test]
    fn sniff_gif_and_webp() {
        assert_eq!(
            detect_supported_image_mime_type(b"GIF89a"),
            Some("image/gif")
        );
        assert_eq!(
            detect_supported_image_mime_type(b"RIFF\x00\x00\x00\x00WEBP"),
            Some("image/webp")
        );
        assert_eq!(detect_supported_image_mime_type(b"plain text"), None);
    }
}
