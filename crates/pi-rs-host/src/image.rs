//! Image normalization used internally by the clipboard effect.

pub(crate) fn convert_bytes_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageOutputFormat::Png)
        .ok()?;
    Some(output.into_inner())
}
