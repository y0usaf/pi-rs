//! Port of cli/file-processor.ts — @file argument processing into
//! initial-message text (PLAN 10 non-interactive argument surface).
//!
//! pi-rs has no image-resize mechanism yet, so image files follow the
//! spec's failure branch: the <file name=...>[Image omitted: could not
//! be resized below the inline image size limit.]</file> note. Text files
//! use the spec's exact <file name="...">\n{content}\n</file>\n framing.

use std::path::Path;

fn is_image_path(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "avif")
    )
}

/// Resolve a @file argument relative to `cwd` with ~ expansion
/// (spec: resolveReadPath + resolve).
pub fn resolve_file_arg(arg: &str, cwd: &Path) -> std::path::PathBuf {
    let expanded = if let Some(rest) = arg.strip_prefix("~/") {
        std::env::var("HOME")
            .map(|home| std::path::PathBuf::from(home).join(rest))
            .unwrap_or_else(|_| std::path::PathBuf::from(arg))
    } else {
        std::path::PathBuf::from(arg)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}

/// Process @file args into initial-message text. Errors carry the
/// spec's messages (File not found: {path},
/// Could not read file {path}: {message}) for stderr + exit 1.
pub fn process_file_arguments(
    file_args: &[String],
    cwd: &Path,
) -> Result<String, String> {
    let mut text = String::new();
    for arg in file_args {
        let absolute = resolve_file_arg(arg, cwd);
        if !absolute.exists() {
            return Err(format!("File not found: {}", absolute.display()));
        }
        if absolute.metadata().map(|m| m.len()).unwrap_or(1) == 0 {
            // Spec: empty files are skipped.
            continue;
        }
        if is_image_path(&absolute) {
            text.push_str(&format!(
                "<file name=\"{}\">[Image omitted: could not be resized below the inline image size limit.]</file>\n",
                absolute.display()
            ));
            continue;
        }
        match std::fs::read_to_string(&absolute) {
            Ok(content) => {
                text.push_str(&format!(
                    "<file name=\"{}\">\n{content}\n</file>\n",
                    absolute.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "Could not read file {}: {error}",
                    absolute.display()
                ));
            }
        }
    }
    Ok(text)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)] // test harness unwraps are fine
mod tests {
    use super::*;

    #[test]
    fn text_file_uses_spec_framing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        let text = process_file_arguments(&["a.txt".to_owned()], dir.path()).unwrap();
        let expected = format!(
            "<file name=\"{}\">\nhello\n</file>\n",
            dir.path().join("a.txt").display()
        );
        assert_eq!(text, expected);
    }

    #[test]
    fn empty_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("empty.txt"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "x").unwrap();
        let text = process_file_arguments(
            &["empty.txt".to_owned(), "b.txt".to_owned()],
            dir.path(),
        )
        .unwrap();
        assert!(text.contains("b.txt"));
        assert!(!text.contains("empty.txt"));
    }

    #[test]
    fn image_file_emits_omitted_note() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pic.png"), "not-a-png").unwrap();
        let text = process_file_arguments(&["pic.png".to_owned()], dir.path()).unwrap();
        assert!(text.contains("Image omitted: could not be resized"));
    }

    #[test]
    fn missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = process_file_arguments(&["nope.txt".to_owned()], dir.path()).unwrap_err();
        assert!(err.starts_with("File not found: "));
    }
}