//! `utils/mime.ts`: which files the read tool sends as images.

pub const IMAGE_TYPE_SNIFF_BYTES: usize = 4100;
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

/// The MIME type of a supported image from its first [`IMAGE_TYPE_SNIFF_BYTES`] bytes.
pub fn detect_supported_image_mime_type(buf: &[u8]) -> Option<&'static str> {
    if buf.starts_with(&[0xff, 0xd8, 0xff]) {
        return if buf.get(3) == Some(&0xf7) {
            None
        } else {
            Some("image/jpeg")
        };
    }
    if buf.starts_with(&PNG_SIGNATURE) {
        return (is_png(buf) && !is_animated_png(buf)).then_some("image/png");
    }
    if buf.starts_with(b"GIF87a") || buf.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if buf.starts_with(b"RIFF") && ascii_at(buf, 8, b"WEBP") {
        return Some("image/webp");
    }
    if buf.starts_with(b"BM") && is_bmp(buf) {
        return Some("image/bmp");
    }
    None
}

/// Sniffs the start of the file at `path`.
pub fn detect_supported_image_mime_type_from_file(
    path: &str,
) -> std::io::Result<Option<&'static str>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(IMAGE_TYPE_SNIFF_BYTES);
    file.take(IMAGE_TYPE_SNIFF_BYTES as u64)
        .read_to_end(&mut buf)?;
    Ok(detect_supported_image_mime_type(&buf))
}

fn ascii_at(buf: &[u8], offset: usize, text: &[u8]) -> bool {
    buf.get(offset..offset + text.len()) == Some(text)
}

fn byte(buf: &[u8], i: usize) -> u64 {
    u64::from(buf.get(i).copied().unwrap_or(0))
}

fn u16_le(buf: &[u8], i: usize) -> u64 {
    byte(buf, i) + (byte(buf, i + 1) << 8)
}

fn u32_be(buf: &[u8], i: usize) -> u64 {
    (byte(buf, i) << 24) + (byte(buf, i + 1) << 16) + (byte(buf, i + 2) << 8) + byte(buf, i + 3)
}

fn u32_le(buf: &[u8], i: usize) -> u64 {
    byte(buf, i) + (byte(buf, i + 1) << 8) + (byte(buf, i + 2) << 16) + (byte(buf, i + 3) << 24)
}

fn is_png(buf: &[u8]) -> bool {
    buf.len() >= 16 && u32_be(buf, 8) == 13 && ascii_at(buf, 12, b"IHDR")
}

fn is_animated_png(buf: &[u8]) -> bool {
    let mut offset = PNG_SIGNATURE.len();
    while offset + 8 <= buf.len() {
        let length = u32_be(buf, offset) as usize;
        if ascii_at(buf, offset + 4, b"acTL") {
            return true;
        }
        if ascii_at(buf, offset + 4, b"IDAT") {
            return false;
        }
        let next = offset + 8 + length + 4;
        if next <= offset || next > buf.len() {
            return false;
        }
        offset = next;
    }
    false
}

fn is_bmp(buf: &[u8]) -> bool {
    if buf.len() < 26 {
        return false;
    }
    let declared = u32_le(buf, 2);
    let pixels = u32_le(buf, 10);
    let dib = u32_le(buf, 14);
    if declared != 0 && declared < 26 {
        return false;
    }
    if pixels < 14 + dib {
        return false;
    }
    if declared != 0 && pixels >= declared {
        return false;
    }
    let (planes, bits) = if dib == 12 {
        (u16_le(buf, 22), u16_le(buf, 24))
    } else if (40..=124).contains(&dib) {
        if buf.len() < 30 {
            return false;
        }
        (u16_le(buf, 26), u16_le(buf, 28))
    } else {
        return false;
    };
    planes == 1 && [1, 4, 8, 16, 24, 32].contains(&bits)
}

/// Standard base64 with padding, `Buffer.toString("base64")`.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads() {
        assert_eq!(base64(b"hi"), "aGk=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64(b"a"), "YQ==");
    }

    #[test]
    fn gif() {
        assert_eq!(
            detect_supported_image_mime_type(b"GIF89a...."),
            Some("image/gif")
        );
        assert_eq!(detect_supported_image_mime_type(b"hello"), None);
    }
}
