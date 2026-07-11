//! Image mechanism — the photon slice pi's coding agent uses, ported 1:1.
//!
//! Pi consumes `@silvia-odwyer/photon-node` 0.3.4 (the WASM build of
//! photon-rs, compiled against `image` 0.24.9 / `png` 0.17.14 /
//! `flate2` 1.0.34 / `miniz_oxide` 0.8.0); pi-rs ports the exact slice
//! over the same pinned `image` crate so encoded bytes match the
//! vendored library byte-for-byte (see `tests/image-parity/`):
//!
//! - `utils/image-resize-core.ts` `resizeImageInProcess` — the read
//!   tool's auto-resize (2000x2000 / 4.5MB base64 cap, PNG-vs-JPEG
//!   candidates, quality steps, 0.75 dimension backoff);
//! - `utils/image-convert.ts` `convertToPng` — kitty-graphics PNG
//!   normalization (base64 in/out, EXIF applied);
//! - `utils/exif-orientation.ts` — JPEG/WebP EXIF orientation parsing
//!   and the flip/rotate corrections;
//! - `clipboard-image.ts` `convertToPng` — bytes-to-PNG for unsupported
//!   clipboard formats (no EXIF pass, matching the spec).
//!
//! Pi runs the resize in a worker thread so WASM work does not block the
//! TUI; pi-rs's async bindings run it on the blocking pool for the same
//! observable behavior (the editor keeps rendering).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageBuffer, RgbaImage};

/// photon-rs `PhotonImage`: raw RGBA8 pixels plus dimensions.
struct PhotonImage {
    raw_pixels: Vec<u8>,
    width: u32,
    height: u32,
}

impl PhotonImage {
    /// photon `PhotonImage::new_from_byteslice` (the spec's TS callers
    /// catch the WASM panic and return null; here it is a Result).
    fn new_from_byteslice(bytes: &[u8]) -> Option<PhotonImage> {
        let img = image::load_from_memory(bytes).ok()?;
        let width = img.width();
        let height = img.height();
        let raw_pixels = img.to_rgba8().into_raw();
        Some(PhotonImage {
            raw_pixels,
            width,
            height,
        })
    }

    fn to_rgba_image(&self) -> Option<RgbaImage> {
        ImageBuffer::from_raw(self.width, self.height, self.raw_pixels.clone())
    }

    /// photon `get_bytes`: PNG-encode (image 0.24.9 defaults —
    /// `CompressionType::Fast`, adaptive filtering).
    fn get_bytes(&self) -> Option<Vec<u8>> {
        let img = DynamicImage::ImageRgba8(self.to_rgba_image()?);
        let mut buffer = std::io::Cursor::new(Vec::new());
        #[allow(deprecated)]
        img.write_to(&mut buffer, image::ImageOutputFormat::Png)
            .ok()?;
        Some(buffer.into_inner())
    }

    /// photon `get_bytes_jpeg(quality)`.
    fn get_bytes_jpeg(&self, quality: u8) -> Option<Vec<u8>> {
        let img = DynamicImage::ImageRgba8(self.to_rgba_image()?);
        let mut buffer = std::io::Cursor::new(Vec::new());
        let mut encoder = JpegEncoder::new_with_quality(&mut buffer, quality);
        encoder.encode_image(&img).ok()?;
        Some(buffer.into_inner())
    }
}

/// photon `transform::resize` (Lanczos3 in every spec call site).
fn photon_resize(img: &PhotonImage, width: u32, height: u32) -> Option<PhotonImage> {
    let dyn_img = DynamicImage::ImageRgba8(img.to_rgba_image()?);
    let resized = image::imageops::resize(&dyn_img, width, height, FilterType::Lanczos3);
    Some(PhotonImage {
        width: resized.width(),
        height: resized.height(),
        raw_pixels: resized.into_raw(),
    })
}

/// photon `transform::fliph` (in-place in photon; by-value here).
fn photon_fliph(img: &mut PhotonImage) {
    let (w, h) = (img.width as usize, img.height as usize);
    for y in 0..h {
        let row = &mut img.raw_pixels[y * w * 4..(y + 1) * w * 4];
        for x in 0..w / 2 {
            for c in 0..4 {
                row.swap(x * 4 + c, (w - 1 - x) * 4 + c);
            }
        }
    }
}

/// photon `transform::flipv`.
fn photon_flipv(img: &mut PhotonImage) {
    let (w, h) = (img.width as usize, img.height as usize);
    let row_len = w * 4;
    for y in 0..h / 2 {
        for i in 0..row_len {
            let a = y * row_len + i;
            let b = (h - 1 - y) * row_len + i;
            img.raw_pixels.swap(a, b);
        }
    }
}

// ---------------------------------------------------------------------------
// exif-orientation.ts
// ---------------------------------------------------------------------------

fn read_orientation_from_tiff(bytes: &[u8], tiff_start: usize) -> u16 {
    if tiff_start + 8 > bytes.len() {
        return 1;
    }
    let byte_order = ((bytes[tiff_start] as u32) << 8) | bytes[tiff_start + 1] as u32;
    let le = byte_order == 0x4949;

    let read16 = |pos: usize| -> u32 {
        if pos + 1 >= bytes.len() {
            return 0;
        }
        if le {
            bytes[pos] as u32 | ((bytes[pos + 1] as u32) << 8)
        } else {
            ((bytes[pos] as u32) << 8) | bytes[pos + 1] as u32
        }
    };
    let read32 = |pos: usize| -> u32 {
        if pos + 3 >= bytes.len() {
            return 0;
        }
        if le {
            bytes[pos] as u32
                | ((bytes[pos + 1] as u32) << 8)
                | ((bytes[pos + 2] as u32) << 16)
                | ((bytes[pos + 3] as u32) << 24)
        } else {
            ((bytes[pos] as u32) << 24)
                | ((bytes[pos + 1] as u32) << 16)
                | ((bytes[pos + 2] as u32) << 8)
                | bytes[pos + 3] as u32
        }
    };

    let ifd_offset = read32(tiff_start + 4) as usize;
    let ifd_start = tiff_start + ifd_offset;
    if ifd_start + 2 > bytes.len() {
        return 1;
    }
    let entry_count = read16(ifd_start) as usize;
    for i in 0..entry_count {
        let entry_pos = ifd_start + 2 + i * 12;
        if entry_pos + 12 > bytes.len() {
            return 1;
        }
        if read16(entry_pos) == 0x0112 {
            let value = read16(entry_pos + 8) as u16;
            return if (1..=8).contains(&value) { value } else { 1 };
        }
    }
    1
}

fn has_exif_header(bytes: &[u8], offset: usize) -> bool {
    bytes.len() > offset + 5
        && bytes[offset] == 0x45
        && bytes[offset + 1] == 0x78
        && bytes[offset + 2] == 0x69
        && bytes[offset + 3] == 0x66
        && bytes[offset + 4] == 0x00
        && bytes[offset + 5] == 0x00
}

fn find_jpeg_tiff_offset(bytes: &[u8]) -> Option<usize> {
    let mut offset = 2usize;
    while offset + 1 < bytes.len() {
        if bytes[offset] != 0xff {
            return None;
        }
        let marker = bytes[offset + 1];
        if marker == 0xff {
            offset += 1;
            continue;
        }
        if marker == 0xe1 {
            if offset + 4 >= bytes.len() {
                return None;
            }
            let segment_start = offset + 4;
            if segment_start + 6 > bytes.len() {
                return None;
            }
            if !has_exif_header(bytes, segment_start) {
                return None;
            }
            return Some(segment_start + 6);
        }
        if offset + 4 > bytes.len() {
            return None;
        }
        let length = ((bytes[offset + 2] as usize) << 8) | bytes[offset + 3] as usize;
        offset += 2 + length;
    }
    None
}

fn find_webp_tiff_offset(bytes: &[u8]) -> Option<usize> {
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = bytes[offset + 4] as usize
            | ((bytes[offset + 5] as usize) << 8)
            | ((bytes[offset + 6] as usize) << 16)
            | ((bytes[offset + 7] as usize) << 24);
        let data_start = offset + 8;
        if chunk_id == b"EXIF" {
            if data_start + chunk_size > bytes.len() {
                return None;
            }
            // Some WebP files have "Exif\0\0" prefix before the TIFF header
            let tiff_start = if chunk_size >= 6 && has_exif_header(bytes, data_start) {
                data_start + 6
            } else {
                data_start
            };
            return Some(tiff_start);
        }
        // RIFF chunks are padded to even size
        offset = data_start + chunk_size + (chunk_size % 2);
    }
    None
}

fn get_exif_orientation(bytes: &[u8]) -> u16 {
    let tiff_offset = if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] == 0xd8 {
        find_jpeg_tiff_offset(bytes)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        find_webp_tiff_offset(bytes)
    } else {
        None
    };
    match tiff_offset {
        Some(offset) => read_orientation_from_tiff(bytes, offset),
        None => 1,
    }
}

/// exif-orientation.ts `rotate90` with the caller's `dstIndex` closure.
fn rotate90(img: &PhotonImage, dst_index: impl Fn(u32, u32, u32, u32) -> u32) -> PhotonImage {
    let (w, h) = (img.width, img.height);
    let src = &img.raw_pixels;
    let mut dst = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            let src_idx = ((y * w + x) * 4) as usize;
            let dst_idx = (dst_index(x, y, w, h) * 4) as usize;
            dst[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }
    PhotonImage {
        raw_pixels: dst,
        width: h,
        height: w,
    }
}

/// exif-orientation.ts `applyExifOrientation`.
fn apply_exif_orientation(mut image: PhotonImage, original_bytes: &[u8]) -> PhotonImage {
    let orientation = get_exif_orientation(original_bytes);
    match orientation {
        2 => {
            photon_fliph(&mut image);
            image
        }
        3 => {
            photon_fliph(&mut image);
            photon_flipv(&mut image);
            image
        }
        4 => {
            photon_flipv(&mut image);
            image
        }
        5 => {
            let mut rotated = rotate90(&image, |x, y, _w, h| x * h + (h - 1 - y));
            photon_fliph(&mut rotated);
            rotated
        }
        6 => rotate90(&image, |x, y, _w, h| x * h + (h - 1 - y)),
        7 => {
            let mut rotated = rotate90(&image, |x, y, w, h| (w - 1 - x) * h + y);
            photon_fliph(&mut rotated);
            rotated
        }
        8 => rotate90(&image, |x, y, w, h| (w - 1 - x) * h + y),
        _ => image,
    }
}

// ---------------------------------------------------------------------------
// image-resize-core.ts
// ---------------------------------------------------------------------------

/// Spec: `ImageResizeOptions` (all optional, JS numbers).
#[derive(Clone, Copy, Debug, Default)]
pub struct ImageResizeOptions {
    pub max_width: Option<f64>,
    pub max_height: Option<f64>,
    pub max_bytes: Option<f64>,
    pub jpeg_quality: Option<f64>,
}

/// Spec: `ResizedImage`.
#[derive(Clone, Debug)]
pub struct ResizedImage {
    /// base64
    pub data: String,
    pub mime_type: String,
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
    pub was_resized: bool,
}

/// 4.5MB of base64 payload. Provides headroom below Anthropic's 5MB limit.
const DEFAULT_MAX_BYTES: f64 = 4.5 * 1024.0 * 1024.0;

struct EncodedCandidate {
    data: String,
    encoded_size: f64,
    mime_type: &'static str,
}

fn encode_candidate(buffer: &[u8], mime_type: &'static str) -> EncodedCandidate {
    let data = BASE64.encode(buffer);
    let encoded_size = data.len() as f64;
    EncodedCandidate {
        data,
        encoded_size,
        mime_type,
    }
}

/// Spec: `resizeImageInProcess` — resize an image to fit within the max
/// dimensions and encoded file size; `None` when it cannot fit below
/// `maxBytes` (or decoding fails, matching the spec's catch-all).
pub fn resize_image(
    input_bytes: &[u8],
    mime_type: &str,
    options: ImageResizeOptions,
) -> Option<ResizedImage> {
    let max_width = options.max_width.unwrap_or(2000.0);
    let max_height = options.max_height.unwrap_or(2000.0);
    let max_bytes = options.max_bytes.unwrap_or(DEFAULT_MAX_BYTES);
    let jpeg_quality = options.jpeg_quality.unwrap_or(80.0);
    let input_base64_size = ((input_bytes.len() as f64) / 3.0).ceil() * 4.0;

    let raw_image = PhotonImage::new_from_byteslice(input_bytes)?;
    let image = apply_exif_orientation(raw_image, input_bytes);

    let original_width = image.width;
    let original_height = image.height;
    let format = mime_type.split('/').nth(1).unwrap_or("png");

    // Check if already within all limits (dimensions AND encoded size)
    if (original_width as f64) <= max_width
        && (original_height as f64) <= max_height
        && input_base64_size < max_bytes
    {
        let mime = if mime_type.is_empty() {
            format!("image/{format}")
        } else {
            mime_type.to_owned()
        };
        return Some(ResizedImage {
            data: BASE64.encode(input_bytes),
            mime_type: mime,
            original_width,
            original_height,
            width: original_width,
            height: original_height,
            was_resized: false,
        });
    }

    // Calculate initial dimensions respecting max limits (JS float math,
    // Math.round == round-half-away-from-zero for positives).
    let mut target_width = original_width as f64;
    let mut target_height = original_height as f64;
    if target_width > max_width {
        target_height = (target_height * max_width / target_width).round();
        target_width = max_width;
    }
    if target_height > max_height {
        target_width = (target_width * max_height / target_height).round();
        target_height = max_height;
    }

    // JS `new Set([...])` — insertion order, deduped.
    let mut quality_steps: Vec<f64> = Vec::new();
    for q in [jpeg_quality, 85.0, 70.0, 55.0, 40.0] {
        if !quality_steps.contains(&q) {
            quality_steps.push(q);
        }
    }

    let try_encodings = |width: f64, height: f64| -> Option<Vec<EncodedCandidate>> {
        let resized = photon_resize(&image, width as u32, height as u32)?;
        let mut candidates = vec![encode_candidate(&resized.get_bytes()?, "image/png")];
        for quality in &quality_steps {
            candidates.push(encode_candidate(
                &resized.get_bytes_jpeg(*quality as u8)?,
                "image/jpeg",
            ));
        }
        Some(candidates)
    };

    let mut current_width = target_width;
    let mut current_height = target_height;
    loop {
        let candidates = try_encodings(current_width, current_height)?;
        for candidate in candidates {
            if candidate.encoded_size < max_bytes {
                return Some(ResizedImage {
                    data: candidate.data,
                    mime_type: candidate.mime_type.to_owned(),
                    original_width,
                    original_height,
                    width: current_width as u32,
                    height: current_height as u32,
                    was_resized: true,
                });
            }
        }

        if current_width == 1.0 && current_height == 1.0 {
            break;
        }
        let next_width = if current_width == 1.0 {
            1.0
        } else {
            (current_width * 0.75).floor().max(1.0)
        };
        let next_height = if current_height == 1.0 {
            1.0
        } else {
            (current_height * 0.75).floor().max(1.0)
        };
        if next_width == current_width && next_height == current_height {
            break;
        }
        current_width = next_width;
        current_height = next_height;
    }
    None
}

/// Spec: `utils/image-convert.ts` `convertToPng` — PNG normalization for
/// terminal display (base64 in/out, EXIF orientation applied).
pub fn convert_to_png_base64(base64_data: &str, mime_type: &str) -> Option<(String, String)> {
    if mime_type == "image/png" {
        return Some((base64_data.to_owned(), mime_type.to_owned()));
    }
    // Buffer.from(base64) never throws; a garbage buffer just fails to
    // decode as an image below (both roads end at null).
    let bytes = BASE64.decode(base64_data.as_bytes()).ok()?;
    let raw_image = PhotonImage::new_from_byteslice(&bytes)?;
    let image = apply_exif_orientation(raw_image, &bytes);
    let png = image.get_bytes()?;
    Some((BASE64.encode(&png), "image/png".to_owned()))
}

/// Spec: `clipboard-image.ts` `convertToPng` — bytes-to-PNG for
/// unsupported clipboard formats (no EXIF pass, matching the spec).
pub(crate) fn convert_bytes_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    PhotonImage::new_from_byteslice(bytes)?.get_bytes()
}
