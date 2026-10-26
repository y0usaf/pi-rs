//! `modes/interactive/external-editor.ts`: the prompt in the user's editor.

use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The prompt after the editor closed, or None when it failed and the prompt stays as it was.
pub fn edit(command: &str, content: &str) -> Option<String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("pi-editor-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&directory).ok()?;
    let path = directory.join("prompt.md");
    let result = edit_file(command, content, &path);
    let _ = std::fs::remove_dir_all(&directory);
    result
}

fn edit_file(command: &str, content: &str, path: &Path) -> Option<String> {
    std::fs::write(path, content).ok()?;
    let mut parts = command.split(' ');
    let editor = parts.next().filter(|editor| !editor.is_empty())?;
    let status = Command::new(editor).args(parts).arg(path).status().ok()?;
    if !status.success() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    Some(match text.strip_suffix('\n') {
        Some(trimmed) => trimmed.to_owned(),
        None => text.to_owned(),
    })
}
